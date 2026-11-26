//! A probe, run by hand (`cargo test -p victory-gpu unified_probe -- --ignored
//! --nocapture`): whether the host GPU can use memory the process already
//! has as a heap, so that title memory would need no copies (upstream issue
//! "one address space for title memory"). Prints what the device answers
//! for each way of getting such a heap; asserts nothing.

use windows::Win32::Foundation::HANDLE;
use windows::Win32::Graphics::Direct3D::D3D_FEATURE_LEVEL_12_0;
use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::core::Interface;
use windows_sys::Win32::System::Memory::{
    CreateFileMappingW, FILE_MAP_ALL_ACCESS, MEM_COMMIT, MEM_RESERVE, MapViewOfFile,
    PAGE_READWRITE, SEC_COMMIT, SEC_RESERVE, VirtualAlloc,
};

/// 64 MiB, or `VICTORY_PROBE_MB` (to see what a run leaves committed).
fn size() -> usize {
    std::env::var("VICTORY_PROBE_MB")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(64)
        << 20
}

fn buffer(size: u64, flags: D3D12_RESOURCE_FLAGS) -> D3D12_RESOURCE_DESC {
    D3D12_RESOURCE_DESC {
        Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
        Width: size,
        Height: 1,
        DepthOrArraySize: 1,
        MipLevels: 1,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
        Flags: flags,
        ..Default::default()
    }
}

struct Gpu {
    device: ID3D12Device,
    queue: ID3D12CommandQueue,
    allocator: ID3D12CommandAllocator,
    list: ID3D12GraphicsCommandList,
    fence: ID3D12Fence,
    value: u64,
}

impl Gpu {
    unsafe fn new() -> Option<Gpu> {
        unsafe {
            let mut device: Option<ID3D12Device> = None;
            // Under RenderDoc if asked for: whether its device takes these calls.
            #[cfg(feature = "diagnostics")]
            crate::renderdoc::load();
            D3D12CreateDevice(None, D3D_FEATURE_LEVEL_12_0, &mut device).ok()?;
            let device = device?;
            let queue: ID3D12CommandQueue = device
                .CreateCommandQueue(&D3D12_COMMAND_QUEUE_DESC::default())
                .ok()?;
            let allocator: ID3D12CommandAllocator = device
                .CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT)
                .ok()?;
            let list: ID3D12GraphicsCommandList = device
                .CreateCommandList(0, D3D12_COMMAND_LIST_TYPE_DIRECT, &allocator, None)
                .ok()?;
            list.Close().ok()?;
            let fence: ID3D12Fence = device.CreateFence(0, D3D12_FENCE_FLAG_NONE).ok()?;
            Some(Gpu {
                device,
                queue,
                allocator,
                list,
                fence,
                value: 0,
            })
        }
    }

    /// Runs `record` on the list and waits for the GPU.
    unsafe fn run(&mut self, record: impl FnOnce(&ID3D12GraphicsCommandList)) -> bool {
        unsafe {
            if self.allocator.Reset().is_err() || self.list.Reset(&self.allocator, None).is_err() {
                return false;
            }
            record(&self.list);
            if self.list.Close().is_err() {
                return false;
            }
            self.queue
                .ExecuteCommandLists(&[Some(self.list.cast().unwrap())]);
            self.value += 1;
            if self.queue.Signal(&self.fence, self.value).is_err() {
                return false;
            }
            let started = std::time::Instant::now();
            while self.fence.GetCompletedValue() < self.value {
                if started.elapsed().as_secs() > 10 {
                    return false;
                }
                std::thread::yield_now();
            }
            self.device.GetDeviceRemovedReason().is_ok()
        }
    }

    unsafe fn committed(&self, kind: D3D12_HEAP_TYPE, size: u64) -> Option<ID3D12Resource> {
        unsafe {
            let mut resource: Option<ID3D12Resource> = None;
            self.device
                .CreateCommittedResource(
                    &D3D12_HEAP_PROPERTIES {
                        Type: kind,
                        ..Default::default()
                    },
                    D3D12_HEAP_FLAG_NONE,
                    &buffer(size, D3D12_RESOURCE_FLAG_NONE),
                    if kind == D3D12_HEAP_TYPE_UPLOAD {
                        D3D12_RESOURCE_STATE_GENERIC_READ
                    } else {
                        D3D12_RESOURCE_STATE_COPY_DEST
                    },
                    None,
                    &mut resource,
                )
                .ok()?;
            resource
        }
    }

    /// What a heap over `memory` (the process's own, `size()` bytes) can do:
    /// buffers placed in it with each set of flags, and whether the GPU and
    /// the CPU see each other's writes through them.
    unsafe fn try_heap(&mut self, name: &str, heap: windows::core::Result<ID3D12Heap>, memory: *mut u8) {
        unsafe {
            let heap = match heap {
                Ok(heap) => heap,
                Err(e) => {
                    println!("{name}: no heap: {e}");
                    return;
                }
            };
            let d = heap.GetDesc();
            println!(
                "{name}: heap of {} bytes, type {:?}, page {:?}, pool {:?}, flags {:#x}",
                d.SizeInBytes,
                d.Properties.Type.0,
                d.Properties.CPUPageProperty.0,
                d.Properties.MemoryPoolPreference.0,
                d.Flags.0
            );
            for (what, flags) in [
                ("cross-adapter", D3D12_RESOURCE_FLAG_ALLOW_CROSS_ADAPTER),
                (
                    "cross-adapter + unordered access",
                    D3D12_RESOURCE_FLAG_ALLOW_CROSS_ADAPTER | D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS,
                ),
                ("no flags", D3D12_RESOURCE_FLAG_NONE),
                ("unordered access", D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS),
            ] {
                let mut placed: Option<ID3D12Resource> = None;
                let made = self.device.CreatePlacedResource(
                    &heap,
                    0,
                    &buffer(size() as u64, flags),
                    D3D12_RESOURCE_STATE_COMMON,
                    None,
                    &mut placed,
                );
                let Some(placed) = placed.filter(|_| made.is_ok()) else {
                    println!("  buffer with {what}: refused: {:?}", made.err());
                    continue;
                };
                println!(
                    "  buffer with {what}: made, GPU address {:#x}",
                    placed.GetGPUVirtualAddress()
                );
                // The GPU reads what the CPU wrote.
                let pattern: Vec<u8> = (0..4096u32).map(|i| (i * 7 + 3) as u8).collect();
                let offset = 5 << 20;
                std::ptr::copy_nonoverlapping(pattern.as_ptr(), memory.add(offset), pattern.len());
                let Some(back) = self.committed(D3D12_HEAP_TYPE_READBACK, 4096) else {
                    println!("    no readback buffer");
                    continue;
                };
                let ran = self.run(|l| l.CopyBufferRegion(&back, 0, &placed, offset as u64, 4096));
                let mut at = std::ptr::null_mut();
                let read = ran
                    && back.Map(0, None, Some(&mut at)).is_ok()
                    && std::slice::from_raw_parts(at.cast::<u8>(), 4096) == pattern.as_slice();
                println!("    GPU reads the CPU's bytes: {read} (ran {ran})");
                // The CPU reads what the GPU wrote.
                let Some(up) = self.committed(D3D12_HEAP_TYPE_UPLOAD, 4096) else {
                    println!("    no upload buffer");
                    continue;
                };
                let mut at = std::ptr::null_mut();
                if up.Map(0, None, Some(&mut at)).is_err() {
                    continue;
                }
                let other: Vec<u8> = (0..4096u32).map(|i| (i * 13 + 5) as u8).collect();
                std::ptr::copy_nonoverlapping(other.as_ptr(), at.cast::<u8>(), 4096);
                let offset = 9 << 20;
                let ran = self.run(|l| l.CopyBufferRegion(&placed, offset as u64, &up, 0, 4096));
                let wrote = ran
                    && std::slice::from_raw_parts(memory.add(offset), 4096) == other.as_slice();
                println!("    CPU reads the GPU's bytes: {wrote} (ran {ran})");
            }
        }
    }
}

#[test]
#[ignore = "a probe of the host device, run by hand"]
fn unified_probe() {
    // SAFETY: plain device, memory and heap calls; the memory outlives the
    // heaps (the process ends with the test).
    unsafe {
        let Some(mut gpu) = Gpu::new() else {
            println!("no host device");
            return;
        };
        let Ok(device3) = gpu.device.cast::<ID3D12Device3>() else {
            println!("no ID3D12Device3");
            return;
        };

        // A section as the runtime makes for the title's physical pages,
        // committed at once.
        let section = CreateFileMappingW(
            windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE,
            std::ptr::null(),
            PAGE_READWRITE | SEC_COMMIT,
            0,
            size() as u32,
            std::ptr::null(),
        );
        let view = MapViewOfFile(section, FILE_MAP_ALL_ACCESS, 0, 0, size()).Value.cast::<u8>();
        println!("committed section {section:?}, view {view:?}");
        let heap = device3.OpenExistingHeapFromFileMapping::<ID3D12Heap>(HANDLE(section));
        gpu.try_heap("from the committed section", heap, view);
        let heap = device3.OpenExistingHeapFromAddress::<ID3D12Heap>(view.cast());
        gpu.try_heap("from the committed section's view address", heap, view);

        // The same, reserved and committed as it is touched (what the
        // runtime does on Windows).
        let section = CreateFileMappingW(
            windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE,
            std::ptr::null(),
            PAGE_READWRITE | SEC_RESERVE,
            0,
            size() as u32,
            std::ptr::null(),
        );
        let view = MapViewOfFile(section, FILE_MAP_ALL_ACCESS, 0, 0, size()).Value.cast::<u8>();
        println!("reserved section {section:?}, view {view:?}");
        let heap = device3.OpenExistingHeapFromFileMapping::<ID3D12Heap>(HANDLE(section));
        if heap.is_ok() {
            // Commit it all before touching it from either side.
            VirtualAlloc(view.cast(), size(), MEM_COMMIT, PAGE_READWRITE);
        }
        gpu.try_heap("from the reserved section", heap, view);

        // Private memory.
        let memory = VirtualAlloc(std::ptr::null(), size(), MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE).cast::<u8>();
        println!("private memory {memory:?}");
        let heap = device3.OpenExistingHeapFromAddress::<ID3D12Heap>(memory.cast());
        gpu.try_heap("from private memory", heap, memory);
        if let Ok(device13) = gpu.device.cast::<ID3D12Device13>() {
            let memory =
                VirtualAlloc(std::ptr::null(), size(), MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE).cast::<u8>();
            // A part of an allocation, as a title buffer inside a larger one.
            let heap = device13.OpenExistingHeapFromAddress1::<ID3D12Heap>(memory.add(16 << 20).cast(), 32 << 20);
            match &heap {
                Ok(h) => println!("part of private memory: heap of {} bytes", h.GetDesc().SizeInBytes),
                Err(e) => println!("part of private memory: no heap: {e}"),
            }
        } else {
            println!("no ID3D12Device13 (OpenExistingHeapFromAddress1)");
        }
    }
}

/// The system's committed bytes.
fn committed() -> u64 {
    use windows_sys::Win32::System::ProcessStatus::{K32GetPerformanceInfo, PERFORMANCE_INFORMATION};
    let mut info = PERFORMANCE_INFORMATION { cb: size_of::<PERFORMANCE_INFORMATION>() as u32, ..Default::default() };
    // SAFETY: a writable structure of the size given.
    unsafe { K32GetPerformanceInfo(&mut info, info.cb) };
    info.CommitTotal as u64 * info.PageSize as u64
}

/// A probe, run by hand: what the system keeps committed after a heap over
/// memory the process has was opened, released, and the memory freed. One
/// case at a time (`VICTORY_PROBE_MB` each): a committed section opened
/// from its handle, the same from its view's address, private memory from
/// its address; each with and without a buffer placed in the heap.
#[test]
#[ignore = "a probe of the host device, run by hand"]
fn unified_leak_probe() {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Memory::{MEM_RELEASE, UnmapViewOfFile, VirtualFree, MEMORY_MAPPED_VIEW_ADDRESS};
    let mib = |v: i64| v / (1 << 20);
    // SAFETY: plain device, memory and heap calls; everything made in a case
    // is released in it.
    unsafe {
        let Some(gpu) = Gpu::new() else {
            println!("no host device");
            return;
        };
        let device3: ID3D12Device3 = gpu.device.cast().unwrap();
        let size = size();
        // The two cases that open the section by its handle keep it
        // committed until the next boot: only with VICTORY_PROBE_LEAKY.
        let leaky = std::env::var_os("VICTORY_PROBE_LEAKY").is_some();
        for (case, place) in [
            ("no heap at all (section made, touched, closed)", false),
            ("section, heap from its handle, no buffer", false),
            ("section, heap from its handle, buffer placed", true),
            ("section, heap from its view's address, buffer placed", true),
            ("private memory, heap from its address, buffer placed", true),
            ("section, heaps from two parts of its view (Address1), buffers placed", true),
        ] {
            if case.contains("from its handle") && !leaky {
                continue;
            }
            let before = committed() as i64;
            let private = case.starts_with("private");
            let (section, memory) = if private {
                let m = VirtualAlloc(std::ptr::null(), size, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE).cast::<u8>();
                (std::ptr::null_mut(), m)
            } else {
                let s = CreateFileMappingW(
                    windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE,
                    std::ptr::null(),
                    PAGE_READWRITE | SEC_COMMIT,
                    0,
                    size as u32,
                    std::ptr::null(),
                );
                (s, MapViewOfFile(s, FILE_MAP_ALL_ACCESS, 0, 0, size).Value.cast::<u8>())
            };
            // Touch every page, as a title would have.
            for at in (0..size).step_by(4096) {
                memory.add(at).write(1);
            }
            let made = committed() as i64;
            {
                let heap: Option<windows::core::Result<ID3D12Heap>> = if case.starts_with("no heap") {
                    None
                } else if case.contains("two parts") {
                    // A quarter of the view from its start, and a part that
                    // overlaps it; the first is dropped here, the second below.
                    let device13: ID3D12Device13 = gpu.device.cast().unwrap();
                    let quarter = (size / 4) & !0xffff;
                    let first = device13.OpenExistingHeapFromAddress1::<ID3D12Heap>(memory.cast(), quarter);
                    println!("  first part: {}", first.as_ref().map_or_else(|e| e.to_string(), |_| "made".into()));
                    Some(device13.OpenExistingHeapFromAddress1(memory.add((quarter / 2) & !0xffff).cast(), quarter))
                } else if case.contains("from its handle") {
                    Some(device3.OpenExistingHeapFromFileMapping(HANDLE(section)))
                } else {
                    Some(device3.OpenExistingHeapFromAddress(memory.cast()))
                };
                if let Some(heap) = heap {
                    match heap {
                        Ok(heap) => {
                            if place {
                                let mut placed: Option<ID3D12Resource> = None;
                                let r = gpu.device.CreatePlacedResource(
                                    &heap,
                                    0,
                                    &buffer(
                                        heap.GetDesc().SizeInBytes,
                                        D3D12_RESOURCE_FLAG_ALLOW_CROSS_ADAPTER
                                            | D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS,
                                    ),
                                    D3D12_RESOURCE_STATE_COMMON,
                                    None,
                                    &mut placed,
                                );
                                if r.is_err() {
                                    println!("  buffer refused: {r:?}");
                                }
                                drop(placed);
                            }
                            drop(heap);
                        }
                        Err(e) => println!("  no heap: {e}"),
                    }
                }
            }
            let released = committed() as i64;
            if private {
                VirtualFree(memory.cast(), 0, MEM_RELEASE);
            } else {
                UnmapViewOfFile(MEMORY_MAPPED_VIEW_ADDRESS { Value: memory.cast() });
                CloseHandle(section);
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
            let freed = committed() as i64;
            println!(
                "{case}: +{} MiB when made; {:+} MiB after the heap was released; left after the memory was freed: {:+} MiB",
                mib(made - before),
                mib(released - made),
                mib(freed - before)
            );
        }
    }
}

/// A probe, run by hand: the process is ended (as `taskkill /F` ends it)
/// with heaps still open over a committed section's view and over private
/// memory, a buffer placed in each. The caller compares the system's
/// committed bytes before and after.
#[test]
#[ignore = "a probe of the host device, run by hand; ends the process"]
fn unified_abandon_probe() {
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, TerminateProcess};
    // SAFETY: plain device, memory and heap calls; nothing is released.
    unsafe {
        let Some(gpu) = Gpu::new() else {
            println!("no host device");
            return;
        };
        let device3: ID3D12Device3 = gpu.device.cast().unwrap();
        let size = size();
        let section = CreateFileMappingW(
            windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE,
            std::ptr::null(),
            PAGE_READWRITE | SEC_COMMIT,
            0,
            size as u32,
            std::ptr::null(),
        );
        let view = MapViewOfFile(section, FILE_MAP_ALL_ACCESS, 0, 0, size).Value.cast::<u8>();
        let private = VirtualAlloc(std::ptr::null(), size, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE).cast::<u8>();
        for memory in [view, private] {
            for at in (0..size).step_by(4096) {
                memory.add(at).write(1);
            }
            let heap: ID3D12Heap = device3.OpenExistingHeapFromAddress(memory.cast()).unwrap();
            let mut placed: Option<ID3D12Resource> = None;
            gpu.device
                .CreatePlacedResource(
                    &heap,
                    0,
                    &buffer(
                        size as u64,
                        D3D12_RESOURCE_FLAG_ALLOW_CROSS_ADAPTER | D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS,
                    ),
                    D3D12_RESOURCE_STATE_COMMON,
                    None,
                    &mut placed,
                )
                .unwrap();
            std::mem::forget(placed);
            std::mem::forget(heap);
        }
        println!("heaps open over a section's view and private memory; ending the process");
        std::mem::forget(gpu);
        TerminateProcess(GetCurrentProcess(), 0);
    }
}

/// A probe, run by hand: a heap over part of a private allocation made as
/// the runtime makes the title's (reserved with write watching, committed
/// in part): whether the host opens it at a 64 KiB and at a 4 KiB start,
/// whether the GPU and the CPU see each other's writes, what freeing the
/// memory under the open heap does, and what stays committed.
#[test]
#[ignore = "a probe of the host device, run by hand"]
fn unified_private_probe() {
    use windows_sys::Win32::System::Memory::{MEM_RELEASE, VirtualFree};
    const MEM_WRITE_WATCH: u32 = 0x20_0000;
    const MIB: usize = 1 << 20;
    let mib = |v: i64| v / (1 << 20);
    // SAFETY: plain device, memory and heap calls.
    unsafe {
        let Some(mut gpu) = Gpu::new() else {
            println!("no host device");
            return;
        };
        let Ok(device13) = gpu.device.cast::<ID3D12Device13>() else {
            println!("no ID3D12Device13 (OpenExistingHeapFromAddress1)");
            return;
        };
        let before = committed() as i64;
        for (case, start, len) in [
            ("64 KiB start, 32 MiB", 4 * MIB, 32 * MIB),
            ("4 KiB start, 32 MiB less 4 KiB", 4 * MIB + 4096, 32 * MIB - 4096),
            ("64 KiB start, 96 KiB", 4 * MIB, 96 << 10),
            ("past the committed part", 4 * MIB, 40 * MIB),
        ] {
            let base = VirtualAlloc(std::ptr::null(), 64 * MIB, MEM_RESERVE | MEM_WRITE_WATCH, PAGE_READWRITE).cast::<u8>();
            // Committed: 4 MiB to 36 MiB of the 64.
            VirtualAlloc(base.add(4 * MIB).cast(), 32 * MIB, MEM_COMMIT, PAGE_READWRITE);
            let memory = base.add(start);
            let heap = device13.OpenExistingHeapFromAddress1::<ID3D12Heap>(memory.cast(), len);
            let heap = match heap {
                Ok(heap) => heap,
                Err(e) => {
                    println!("{case}: no heap: {e}");
                    VirtualFree(base.cast(), 0, MEM_RELEASE);
                    continue;
                }
            };
            let size = heap.GetDesc().SizeInBytes;
            let mut placed: Option<ID3D12Resource> = None;
            let made = gpu.device.CreatePlacedResource(
                &heap,
                0,
                &buffer(
                    size,
                    D3D12_RESOURCE_FLAG_ALLOW_CROSS_ADAPTER | D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS,
                ),
                D3D12_RESOURCE_STATE_COMMON,
                None,
                &mut placed,
            );
            let Some(placed) = placed.filter(|_| made.is_ok()) else {
                println!("{case}: heap of {size} bytes, buffer refused: {:?}", made.err());
                VirtualFree(base.cast(), 0, MEM_RELEASE);
                continue;
            };
            // The GPU reads what the CPU wrote, near the end of the range.
            let at = (len - 8192) & !0xfff;
            let pattern: Vec<u8> = (0..4096u32).map(|i| (i * 7 + 3) as u8).collect();
            std::ptr::copy_nonoverlapping(pattern.as_ptr(), memory.add(at), 4096);
            let back = gpu.committed(D3D12_HEAP_TYPE_READBACK, 4096).unwrap();
            let ran = gpu.run(|l| l.CopyBufferRegion(&back, 0, &placed, at as u64, 4096));
            let mut p = std::ptr::null_mut();
            let read = ran
                && back.Map(0, None, Some(&mut p)).is_ok()
                && std::slice::from_raw_parts(p.cast::<u8>(), 4096) == pattern.as_slice();
            // The CPU reads what the GPU wrote.
            let up = gpu.committed(D3D12_HEAP_TYPE_UPLOAD, 4096).unwrap();
            let mut p = std::ptr::null_mut();
            up.Map(0, None, Some(&mut p)).unwrap();
            let other: Vec<u8> = (0..4096u32).map(|i| (i * 13 + 5) as u8).collect();
            std::ptr::copy_nonoverlapping(other.as_ptr(), p.cast::<u8>(), 4096);
            let ran = gpu.run(|l| l.CopyBufferRegion(&placed, 0, &up, 0, 4096));
            let wrote = ran && std::slice::from_raw_parts(memory, 4096) == other.as_slice();
            // The title frees the memory while the heap is open; the GPU
            // then reads the range again.
            let freed = VirtualFree(base.cast(), 0, MEM_RELEASE) != 0;
            let after_free = gpu.run(|l| l.CopyBufferRegion(&back, 0, &placed, at as u64, 4096));
            println!(
                "{case}: heap of {size} bytes; GPU reads the CPU's bytes: {read}; CPU reads the GPU's: {wrote}; \
                 freed under the open heap: {freed}; a GPU read after that ran: {after_free}"
            );
            drop(placed);
            drop(heap);
        }
        // Two heaps over the same memory at once (a second, larger one made
        // while lists still hold the first), then decommitting and freeing:
        // under an open heap, and once both are released.
        {
            use windows_sys::Win32::System::Memory::MEM_DECOMMIT;
            let base = VirtualAlloc(std::ptr::null(), 64 * MIB, MEM_RESERVE | MEM_WRITE_WATCH, PAGE_READWRITE).cast::<u8>();
            VirtualAlloc(base.cast(), 48 * MIB, MEM_COMMIT, PAGE_READWRITE);
            let first = device13.OpenExistingHeapFromAddress1::<ID3D12Heap>(base.cast(), 16 * MIB);
            let second = device13.OpenExistingHeapFromAddress1::<ID3D12Heap>(base.cast(), 32 * MIB);
            println!(
                "two heaps over the same memory: first {}, second {}",
                first.as_ref().map_or_else(|e| e.to_string(), |_| "made".into()),
                second.as_ref().map_or_else(|e| e.to_string(), |_| "made".into())
            );
            let outside = VirtualFree(base.add(40 * MIB).cast(), 4 * MIB, MEM_DECOMMIT) != 0;
            let inside = VirtualFree(base.add(8 * MIB).cast(), 4 * MIB, MEM_DECOMMIT) != 0;
            println!("decommit under open heaps: outside them {outside}, inside them {inside}");
            drop(first);
            let one_left = VirtualFree(base.cast(), 0, MEM_RELEASE) != 0;
            drop(second);
            let none_left = VirtualFree(base.cast(), 0, MEM_RELEASE) != 0;
            println!("free: with one heap still open {one_left}; with both released {none_left}");
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
        println!("left committed after all cases: {:+} MiB", mib(committed() as i64 - before));
    }
}

/// A probe, run by hand: a heap over a run of the title's addresses whose
/// pages are views of a section in another order (64 KiB views mapped
/// over placeholders, as the runtime maps the title's physical pages):
/// whether the host opens one heap across several views, whether the GPU
/// then sees the pages in the order of the addresses, what unmapping and
/// remapping a view under the open heap does, and what stays committed.
#[test]
#[ignore = "a probe of the host device, run by hand"]
fn unified_views_probe() {
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::System::Memory::{
        MEM_PRESERVE_PLACEHOLDER, MEM_RELEASE, MEM_REPLACE_PLACEHOLDER, MEM_RESERVE_PLACEHOLDER,
        MEMORY_MAPPED_VIEW_ADDRESS, MapViewOfFile3, PAGE_NOACCESS, UnmapViewOfFile2, VirtualAlloc2, VirtualFree,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    const PAGE: usize = 64 << 10;
    const VIEWS: usize = 4;
    let mib = |v: i64| v / (1 << 20);
    // SAFETY: plain device, memory and heap calls on memory made here.
    unsafe {
        let Some(mut gpu) = Gpu::new() else {
            println!("no host device");
            return;
        };
        let Ok(device13) = gpu.device.cast::<ID3D12Device13>() else {
            println!("no ID3D12Device13");
            return;
        };
        let before = committed() as i64;
        let process = GetCurrentProcess();
        let section = CreateFileMappingW(INVALID_HANDLE_VALUE, std::ptr::null(), PAGE_READWRITE | SEC_COMMIT, 0, (16 * PAGE) as u32, std::ptr::null());
        // A placeholder of four pages, split into four, each then a view of
        // another page of the section.
        let base = VirtualAlloc2(process, std::ptr::null(), VIEWS * PAGE, MEM_RESERVE | MEM_RESERVE_PLACEHOLDER, PAGE_NOACCESS, std::ptr::null_mut(), 0).cast::<u8>();
        for k in 0..VIEWS - 1 {
            VirtualFree(base.add(k * PAGE).cast(), PAGE, MEM_RELEASE | MEM_PRESERVE_PLACEHOLDER);
        }
        let order = [5usize, 2, 9, 3];
        let map = |k: usize, page: usize| {
            MapViewOfFile3(section, process, base.add(k * PAGE).cast(), (page * PAGE) as u64, PAGE, MEM_REPLACE_PLACEHOLDER, PAGE_READWRITE, std::ptr::null_mut(), 0)
        };
        for (k, &page) in order.iter().enumerate() {
            let view = map(k, page);
            if view.Value.is_null() {
                println!("view {k} not mapped: {}", std::io::Error::last_os_error());
                return;
            }
            for i in 0..PAGE / 4 {
                base.add(k * PAGE + 4 * i).cast::<u32>().write((page as u32) << 24 | i as u32);
            }
        }
        let read_all = |gpu: &mut Gpu, placed: &ID3D12Resource| -> Option<Vec<u32>> {
            let back = gpu.committed(D3D12_HEAP_TYPE_READBACK, (VIEWS * PAGE) as u64)?;
            if !gpu.run(|l| l.CopyBufferRegion(&back, 0, placed, 0, (VIEWS * PAGE) as u64)) {
                return None;
            }
            let mut p = std::ptr::null_mut();
            back.Map(0, None, Some(&mut p)).ok()?;
            Some(std::slice::from_raw_parts(p.cast::<u32>(), VIEWS * PAGE / 4).to_vec())
        };
        let pages = |words: &[u32]| -> Vec<u32> { (0..VIEWS).map(|k| words[k * PAGE / 4] >> 24).collect() };
        for (what, len) in [("one view", PAGE), ("two views", 2 * PAGE), ("four views", VIEWS * PAGE)] {
            match device13.OpenExistingHeapFromAddress1::<ID3D12Heap>(base.cast(), len) {
                Ok(h) => println!("heap over {what}: made, {} bytes", h.GetDesc().SizeInBytes),
                Err(e) => println!("heap over {what}: no heap: {e}"),
            }
        }
        let Ok(heap) = device13.OpenExistingHeapFromAddress1::<ID3D12Heap>(base.cast(), VIEWS * PAGE) else {
            println!("no heap across the views: nothing more to probe");
            return;
        };
        let mut placed: Option<ID3D12Resource> = None;
        let made = gpu.device.CreatePlacedResource(
            &heap,
            0,
            &buffer((VIEWS * PAGE) as u64, D3D12_RESOURCE_FLAG_ALLOW_CROSS_ADAPTER | D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS),
            D3D12_RESOURCE_STATE_COMMON,
            None,
            &mut placed,
        );
        let Some(placed) = placed.filter(|_| made.is_ok()) else {
            println!("buffer refused: {:?}", made.err());
            return;
        };
        match read_all(&mut gpu, &placed) {
            Some(words) => println!("the GPU sees the section's pages {:?} (the addresses hold {order:?})", pages(&words)),
            None => println!("the GPU read did not run"),
        }
        // The title unmaps the second view and maps another page there.
        let unmapped = UnmapViewOfFile2(process, MEMORY_MAPPED_VIEW_ADDRESS { Value: base.add(PAGE).cast() }, MEM_PRESERVE_PLACEHOLDER) != 0;
        println!("unmapping a view under the open heap: {unmapped} ({})", if unmapped { "ok".to_string() } else { std::io::Error::last_os_error().to_string() });
        if unmapped {
            let again = map(1, 12);
            println!("mapping another page there: {}", !again.Value.is_null());
            if !again.Value.is_null() {
                for i in 0..PAGE / 4 {
                    base.add(PAGE + 4 * i).cast::<u32>().write(12 << 24 | i as u32);
                }
            }
            match read_all(&mut gpu, &placed) {
                Some(words) => println!("after the remap the GPU sees pages {:?} through the old heap (the addresses hold [5, 12, 9, 3])", pages(&words)),
                None => println!("the GPU read after the remap did not run"),
            }
        }
        drop(placed);
        drop(heap);
        std::thread::sleep(std::time::Duration::from_millis(300));
        println!("left committed with everything still mapped: {:+} MiB (the section is 1 MiB)", mib(committed() as i64 - before));
    }
}

/// Title-like view memory for the stretch probes: a section made reserved,
/// as the runtime makes the title's physical pages, shown at a run of
/// addresses one 64 KiB view at a time over placeholders (as
/// `XMemMapPhysicalPages` maps them), each view's page committed and
/// touched. Returns the section and the base address.
unsafe fn title_views(size: usize) -> (windows_sys::Win32::Foundation::HANDLE, *mut u8) {
    use windows_sys::Win32::System::Memory::{
        MEM_PRESERVE_PLACEHOLDER, MEM_RELEASE, MEM_REPLACE_PLACEHOLDER, MEM_RESERVE_PLACEHOLDER, MapViewOfFile3,
        PAGE_NOACCESS, VirtualAlloc2, VirtualFree,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    const PAGE: usize = 64 << 10;
    // SAFETY: memory and views made here, for the caller to release.
    unsafe {
        let process = GetCurrentProcess();
        let section = CreateFileMappingW(
            windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE,
            std::ptr::null(),
            PAGE_READWRITE | SEC_RESERVE,
            (size as u64 >> 32) as u32,
            size as u32,
            std::ptr::null(),
        );
        assert!(!section.is_null(), "no section: {}", std::io::Error::last_os_error());
        let base = VirtualAlloc2(
            process,
            std::ptr::null(),
            size,
            MEM_RESERVE | MEM_RESERVE_PLACEHOLDER,
            PAGE_NOACCESS,
            std::ptr::null_mut(),
            0,
        )
        .cast::<u8>();
        assert!(!base.is_null());
        for k in 0..size / PAGE - 1 {
            VirtualFree(base.add(k * PAGE).cast(), PAGE, MEM_RELEASE | MEM_PRESERVE_PLACEHOLDER);
        }
        for k in 0..size / PAGE {
            let view = MapViewOfFile3(
                section,
                process,
                base.add(k * PAGE).cast(),
                (k * PAGE) as u64,
                PAGE,
                MEM_REPLACE_PLACEHOLDER,
                PAGE_READWRITE,
                std::ptr::null_mut(),
                0,
            );
            assert!(!view.Value.is_null(), "view {k}: {}", std::io::Error::last_os_error());
            VirtualAlloc(base.add(k * PAGE).cast(), PAGE, MEM_COMMIT, PAGE_READWRITE);
            for at in (0..PAGE).step_by(4096) {
                base.add(k * PAGE + at).write(1);
            }
        }
        (section, base)
    }
}

/// A probe, run by hand: exactly the path of `VICTORY_STRETCH=1`
/// (`crate::inplace::place_stretch`: a view of ours mapped on a committed
/// stretch of a reserved section, a heap opened on it with
/// `OpenExistingHeapFromAddress1`, a buffer placed over it), released in
/// the order the code releases it (the buffer, the heap, then our view),
/// then the title's views and the section; three rounds of
/// `VICTORY_PROBE_MB` each. Prints the system's committed bytes at each
/// step, and whether the GPU and the title's view see each other's bytes.
#[test]
#[ignore = "a probe of the host device, run by hand"]
fn unified_stretch_probe() {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Memory::{MEMORY_MAPPED_VIEW_ADDRESS, UnmapViewOfFile2};
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    const PAGE: usize = 64 << 10;
    let mib = |v: i64| v / (1 << 20);
    // SAFETY: plain device, memory and heap calls on memory made here.
    unsafe {
        let Some(mut gpu) = Gpu::new() else {
            println!("no host device");
            return;
        };
        if gpu.device.cast::<ID3D12Device13>().is_err() {
            println!("no ID3D12Device13 (OpenExistingHeapFromAddress1)");
            return;
        }
        let size = size();
        let start_all = committed() as i64;
        for round in 1..=3u32 {
            let before = committed() as i64;
            let (section, base) = title_views(size);
            let made = committed() as i64;
            let (start, len) = (base as u64 + 0x1230, (size - 2 * PAGE) as u64);
            let placed = match crate::inplace::place_stretch(&gpu.device, start, len, section as usize, 0x1230) {
                Ok(placed) => placed,
                Err(e) => {
                    println!("round {round}: not placed: {e:?}");
                    return;
                }
            };
            let open = committed() as i64;
            // The GPU reads what the title wrote at its own views (across a
            // view boundary), and the title sees what the GPU wrote.
            let at = 3 * PAGE as u64 - 2048;
            let pattern: Vec<u8> = (0..4096u32).map(|i| (i * 7 + 3 + round) as u8).collect();
            std::ptr::copy_nonoverlapping(pattern.as_ptr(), (start + at) as *mut u8, 4096);
            let back = gpu.committed(D3D12_HEAP_TYPE_READBACK, 4096).unwrap();
            let ran = gpu.run(|l| l.CopyBufferRegion(&back, 0, &placed.buffer, placed.bias + at, 4096));
            let mut p = std::ptr::null_mut();
            let read = ran
                && back.Map(0, None, Some(&mut p)).is_ok()
                && std::slice::from_raw_parts(p.cast::<u8>(), 4096) == pattern.as_slice();
            let up = gpu.committed(D3D12_HEAP_TYPE_UPLOAD, 4096).unwrap();
            let mut p = std::ptr::null_mut();
            up.Map(0, None, Some(&mut p)).unwrap();
            let other: Vec<u8> = (0..4096u32).map(|i| (i * 13 + 5) as u8).collect();
            std::ptr::copy_nonoverlapping(other.as_ptr(), p.cast::<u8>(), 4096);
            let at2 = 7 * PAGE as u64 + 100;
            let ran2 = gpu.run(|l| l.CopyBufferRegion(&placed.buffer, placed.bias + at2, &up, 0, 4096));
            let wrote = ran2 && std::slice::from_raw_parts((start + at2) as *const u8, 4096) == other.as_slice();
            drop(back);
            drop(up);
            // As the copy holding it drops it: the buffer, the heap, our view.
            drop(placed);
            std::thread::sleep(std::time::Duration::from_millis(500));
            let released = committed() as i64;
            let ours = crate::inplace::own_views_mapped();
            let process = GetCurrentProcess();
            for k in 0..size / PAGE {
                UnmapViewOfFile2(process, MEMORY_MAPPED_VIEW_ADDRESS { Value: base.add(k * PAGE).cast() }, 0);
            }
            CloseHandle(section);
            std::thread::sleep(std::time::Duration::from_millis(500));
            let freed = committed() as i64;
            println!(
                "stretch round {round}: GPU reads the title's bytes {read} (ran {ran}); the title sees the GPU's {wrote} \
                 (ran {ran2}); +{} MiB when the title's views were made; {:+} MiB with the stretch open; {:+} MiB after \
                 it was released (our views still mapped: {ours}); left after the title's views and the section went: \
                 {:+} MiB",
                mib(made - before),
                mib(open - made),
                mib(released - made),
                mib(freed - before)
            );
        }
        println!("stretch probe: left committed after all rounds: {:+} MiB", mib(committed() as i64 - start_all));
    }
}

/// A probe, run by hand: the process is ended (as `taskkill /F` ends it)
/// with a stretch open as `VICTORY_STRETCH=1` opens it (our view, the heap
/// on it, a buffer placed), the title's views and section still there. The
/// caller compares the system's committed bytes before and after.
#[test]
#[ignore = "a probe of the host device, run by hand; ends the process"]
fn unified_stretch_abandon_probe() {
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, TerminateProcess};
    // SAFETY: plain device, memory and heap calls; nothing is released.
    unsafe {
        let Some(gpu) = Gpu::new() else {
            println!("no host device");
            return;
        };
        let size = size();
        let (section, base) = title_views(size);
        let placed = crate::inplace::place_stretch(&gpu.device, base as u64, size as u64, section as usize, 0);
        println!("stretch open: {}; ending the process", placed.is_ok());
        std::mem::forget(placed);
        std::mem::forget(gpu);
        TerminateProcess(GetCurrentProcess(), 0);
    }
}
