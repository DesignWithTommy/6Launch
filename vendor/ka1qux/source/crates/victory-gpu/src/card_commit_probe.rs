//! A probe, run by hand (`cargo test -p victory-gpu card_commit_probe --
//! --ignored --nocapture`): what the card's own memory costs the process in
//! the PC's commit. Makes resources in the card's memory (heap type
//! default) a few ways and prints, against before: the process's private
//! bytes, its committed write-combined private regions (what the memory
//! map line of `copystat` calls "write-combined"), the system's commit and
//! the card's memory in use. Asserts nothing.

use windows::Win32::Graphics::Direct3D::D3D_FEATURE_LEVEL_12_0;
use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::core::Interface;

/// (process private bytes, write-combined private committed bytes, the
/// system's commit, the card's memory in use by the process).
fn measure(adapter: Option<&IDXGIAdapter3>) -> [i64; 4] {
    use windows_sys::Win32::System::Memory::{
        MEM_COMMIT, MEM_PRIVATE, MEMORY_BASIC_INFORMATION, PAGE_WRITECOMBINE, VirtualQuery,
    };
    use windows_sys::Win32::System::ProcessStatus::{
        K32GetPerformanceInfo, K32GetProcessMemoryInfo, PERFORMANCE_INFORMATION, PROCESS_MEMORY_COUNTERS,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    let mut c = PROCESS_MEMORY_COUNTERS { cb: size_of::<PROCESS_MEMORY_COUNTERS>() as u32, ..Default::default() };
    // SAFETY: our own process and a sized structure.
    unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb) };
    let mut info = PERFORMANCE_INFORMATION { cb: size_of::<PERFORMANCE_INFORMATION>() as u32, ..Default::default() };
    // SAFETY: a writable structure of the size given.
    unsafe { K32GetPerformanceInfo(&mut info, info.cb) };
    let mut wc = 0u64;
    let mut at = 0usize;
    loop {
        // SAFETY: plain data filled in by the query.
        let mut m: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: a valid out pointer of the right size.
        if unsafe { VirtualQuery(at as *const _, &mut m, size_of_val(&m)) } == 0 {
            break;
        }
        if m.State == MEM_COMMIT && m.Type == MEM_PRIVATE && m.Protect & PAGE_WRITECOMBINE != 0 {
            wc += m.RegionSize as u64;
        }
        let next = (m.BaseAddress as usize).saturating_add(m.RegionSize);
        if next <= at {
            break;
        }
        at = next;
    }
    let card = adapter.map_or(0, |a| {
        let mut own = DXGI_QUERY_VIDEO_MEMORY_INFO::default();
        // SAFETY: a plain query into our structure.
        unsafe { a.QueryVideoMemoryInfo(0, DXGI_MEMORY_SEGMENT_GROUP_LOCAL, &mut own) }.map_or(0, |()| own.CurrentUsage)
    });
    [
        c.PagefileUsage as i64,
        wc as i64,
        (info.CommitTotal * info.PageSize) as i64,
        card as i64,
    ]
}

fn texture(width: u64, height: u32, flags: D3D12_RESOURCE_FLAGS) -> D3D12_RESOURCE_DESC {
    D3D12_RESOURCE_DESC {
        Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
        Width: width,
        Height: height,
        DepthOrArraySize: 1,
        MipLevels: 1,
        Format: DXGI_FORMAT_R8G8B8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Flags: flags,
        ..Default::default()
    }
}

fn print(case: &str, before: [i64; 4], after: [i64; 4], freed: [i64; 4]) {
    let mb = |v: i64| v / (1 << 20);
    println!(
        "{case}: made: private {:+} MB, write-combined {:+} MB, system commit {:+} MB, card {:+} MB; released: private {:+} MB, write-combined {:+} MB, system commit {:+} MB, card {:+} MB",
        mb(after[0] - before[0]),
        mb(after[1] - before[1]),
        mb(after[2] - before[2]),
        mb(after[3] - before[3]),
        mb(freed[0] - before[0]),
        mb(freed[1] - before[1]),
        mb(freed[2] - before[2]),
        mb(freed[3] - before[3]),
    );
}

#[test]
#[ignore = "a probe of the host device, run by hand"]
fn card_commit_probe() {
    // SAFETY: plain device and resource calls; everything made in a case is
    // released in it.
    unsafe {
        let factory: IDXGIFactory4 = CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0)).unwrap();
        let adapter: IDXGIAdapter1 = factory.EnumAdapters1(0).unwrap();
        let adapter3: IDXGIAdapter3 = adapter.cast().unwrap();
        let mut device: Option<ID3D12Device> = None;
        if D3D12CreateDevice(&adapter, D3D_FEATURE_LEVEL_12_0, &mut device).is_err() {
            println!("no host device");
            return;
        }
        let device = device.unwrap();
        let default = D3D12_HEAP_PROPERTIES { Type: D3D12_HEAP_TYPE_DEFAULT, ..Default::default() };
        let commit = |desc: &D3D12_RESOURCE_DESC, flags: D3D12_HEAP_FLAGS| -> Option<ID3D12Resource> {
            let mut out: Option<ID3D12Resource> = None;
            device
                .CreateCommittedResource(&default, flags, desc, D3D12_RESOURCE_STATE_COMMON, None, &mut out)
                .ok()?;
            out
        };
        // 1 GB a case. `VICTORY_PROBE_CASE=<n>` runs case n alone (a fresh
        // process each: the driver keeps blocks of an earlier case).
        let only = std::env::var("VICTORY_PROBE_CASE").ok().and_then(|v| v.parse::<usize>().ok());
        let cases: [(&str, D3D12_RESOURCE_DESC, usize, D3D12_HEAP_FLAGS); 7] = [
            ("4096 textures 256x256 (256 KB each)", texture(256, 256, D3D12_RESOURCE_FLAG_NONE), 4096, D3D12_HEAP_FLAG_NONE),
            ("1024 textures 512x512 (1 MB each)", texture(512, 512, D3D12_RESOURCE_FLAG_NONE), 1024, D3D12_HEAP_FLAG_NONE),
            ("256 textures 1024x1024 (4 MB each)", texture(1024, 1024, D3D12_RESOURCE_FLAG_NONE), 256, D3D12_HEAP_FLAG_NONE),
            ("64 textures 2048x2048 (16 MB each)", texture(2048, 2048, D3D12_RESOURCE_FLAG_NONE), 64, D3D12_HEAP_FLAG_NONE),
            ("64 render targets 2048x2048 (16 MB each)", texture(2048, 2048, D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET), 64, D3D12_HEAP_FLAG_NONE),
            (
                "256 textures 1024x1024 made not resident",
                texture(1024, 1024, D3D12_RESOURCE_FLAG_NONE),
                256,
                D3D12_HEAP_FLAG_CREATE_NOT_RESIDENT,
            ),
            ("16384 textures 128x128 (64 KB each)", texture(128, 128, D3D12_RESOURCE_FLAG_NONE), 16384, D3D12_HEAP_FLAG_NONE),
        ];
        for (i, (case, desc, n, flags)) in cases.into_iter().enumerate() {
            if only.is_some_and(|o| o != i) {
                continue;
            }
            let before = measure(Some(&adapter3));
            let made: Vec<ID3D12Resource> = (0..n).filter_map(|_| commit(&desc, flags)).collect();
            let after = measure(Some(&adapter3));
            let count = made.len();
            drop(made);
            let freed = measure(Some(&adapter3));
            print(&format!("{i}: {case} ({count} made)"), before, after, freed);
            // Again: what the blocks kept from the first time take back.
            let made: Vec<ID3D12Resource> = (0..n).filter_map(|_| commit(&desc, flags)).collect();
            let again = measure(Some(&adapter3));
            drop(made);
            print(&format!("{i}:   made again"), before, again, measure(Some(&adapter3)));
        }
        if only.is_some() {
            return;
        }
        // Made not resident, then made resident; and made resident, then
        // evicted: whether the commit follows residency.
        let tex = texture(1024, 1024, D3D12_RESOURCE_FLAG_NONE);
        let before = measure(Some(&adapter3));
        let made: Vec<ID3D12Resource> = (0..64).filter_map(|_| commit(&tex, D3D12_HEAP_FLAG_CREATE_NOT_RESIDENT)).collect();
        let pageable: Vec<Option<ID3D12Pageable>> = made.iter().map(|r| Some(r.cast().unwrap())).collect();
        let _ = device.MakeResident(&pageable);
        let after = measure(Some(&adapter3));
        let _ = device.Evict(&pageable);
        let evicted = measure(Some(&adapter3));
        drop(pageable);
        drop(made);
        print("64 textures of 4 MB made not resident, then made resident", before, after, measure(Some(&adapter3)));
        print("  the same, made resident then evicted (\"released\" is after the eviction)", before, after, evicted);
        let before = measure(Some(&adapter3));
        let made: Vec<ID3D12Resource> = (0..64).filter_map(|_| commit(&tex, D3D12_HEAP_FLAG_NONE)).collect();
        let after = measure(Some(&adapter3));
        let pageable: Vec<Option<ID3D12Pageable>> = made.iter().map(|r| Some(r.cast().unwrap())).collect();
        let _ = device.Evict(&pageable);
        let evicted = measure(Some(&adapter3));
        drop(pageable);
        drop(made);
        print("64 textures of 4 MB made resident, then evicted (\"released\" is after the eviction)", before, after, evicted);
        // A buffer and a heap of the card's own memory, 256 MB each.
        let size = 256u64 << 20;
        let buffer = D3D12_RESOURCE_DESC {
            Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
            Width: size,
            Height: 1,
            DepthOrArraySize: 1,
            MipLevels: 1,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
            ..Default::default()
        };
        let before = measure(Some(&adapter3));
        let one = commit(&buffer, D3D12_HEAP_FLAG_NONE);
        let after = measure(Some(&adapter3));
        drop(one);
        print("a buffer of 256 MB", before, after, measure(Some(&adapter3)));
        let before = measure(Some(&adapter3));
        let mut heap: Option<ID3D12Heap> = None;
        let desc = D3D12_HEAP_DESC {
            SizeInBytes: size,
            Properties: default,
            Alignment: 0,
            Flags: D3D12_HEAP_FLAG_ALLOW_ONLY_NON_RT_DS_TEXTURES,
        };
        let _ = device.CreateHeap(&desc, &mut heap);
        let mut placed = Vec::new();
        if let Some(heap) = &heap {
            let tex = texture(1024, 1024, D3D12_RESOURCE_FLAG_NONE);
            for i in 0..64u64 {
                let mut out: Option<ID3D12Resource> = None;
                if device
                    .CreatePlacedResource(heap, i * (4 << 20), &tex, D3D12_RESOURCE_STATE_COMMON, None, &mut out)
                    .is_ok()
                {
                    placed.push(out);
                }
            }
        }
        let after = measure(Some(&adapter3));
        let count = placed.len();
        drop(placed);
        drop(heap);
        print(&format!("a heap of 256 MB with {count} textures of 4 MB placed"), before, after, measure(Some(&adapter3)));
    }
}
