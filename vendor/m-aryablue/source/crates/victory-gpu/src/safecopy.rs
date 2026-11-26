//! fault-safe copy of our own memory without the kernel (VICTORY_FAST_READ)

use std::sync::OnceLock;

// leaf function, volatile registers only, no stack use: needs no unwind data
std::arch::global_asm!(
    ".text",
    ".p2align 4",
    ".globl victory_safe_copy",
    "victory_safe_copy:",
    "    cmp r8, 64",
    "    jb victory_safe_copy_16",
    "victory_safe_copy_64:",
    "    movdqu xmm0, xmmword ptr [rdx]",
    "    movdqu xmm1, xmmword ptr [rdx + 16]",
    "    movdqu xmm2, xmmword ptr [rdx + 32]",
    "    movdqu xmm3, xmmword ptr [rdx + 48]",
    "    movdqu xmmword ptr [rcx], xmm0",
    "    movdqu xmmword ptr [rcx + 16], xmm1",
    "    movdqu xmmword ptr [rcx + 32], xmm2",
    "    movdqu xmmword ptr [rcx + 48], xmm3",
    "    add rdx, 64",
    "    add rcx, 64",
    "    sub r8, 64",
    "    cmp r8, 64",
    "    jae victory_safe_copy_64",
    "victory_safe_copy_16:",
    "    cmp r8, 16",
    "    jb victory_safe_copy_1",
    "    movdqu xmm0, xmmword ptr [rdx]",
    "    movdqu xmmword ptr [rcx], xmm0",
    "    add rdx, 16",
    "    add rcx, 16",
    "    sub r8, 16",
    "    jmp victory_safe_copy_16",
    "victory_safe_copy_1:",
    "    test r8, r8",
    "    jz victory_safe_copy_done",
    "    movzx eax, byte ptr [rdx]",
    "    mov byte ptr [rcx], al",
    "    inc rdx",
    "    inc rcx",
    "    dec r8",
    "    jmp victory_safe_copy_1",
    "victory_safe_copy_done:",
    "    mov eax, 1",
    "    ret",
    // streaming stores past the cache, dst aligned to 16 (VICTORY_COPY_STREAM)
    ".globl victory_safe_copy_nt",
    "victory_safe_copy_nt:",
    "    cmp r8, 256",
    "    jb victory_safe_copy",
    "    mov rax, rcx",
    "    neg rax",
    "    and rax, 15",
    "    jz victory_safe_copy_nt_64",
    "    movdqu xmm0, xmmword ptr [rdx]",
    "    movdqu xmmword ptr [rcx], xmm0",
    "    add rcx, rax",
    "    add rdx, rax",
    "    sub r8, rax",
    "victory_safe_copy_nt_64:",
    "    cmp r8, 64",
    "    jb victory_safe_copy_nt_tail",
    "    movdqu xmm0, xmmword ptr [rdx]",
    "    movdqu xmm1, xmmword ptr [rdx + 16]",
    "    movdqu xmm2, xmmword ptr [rdx + 32]",
    "    movdqu xmm3, xmmword ptr [rdx + 48]",
    "    movntdq xmmword ptr [rcx], xmm0",
    "    movntdq xmmword ptr [rcx + 16], xmm1",
    "    movntdq xmmword ptr [rcx + 32], xmm2",
    "    movntdq xmmword ptr [rcx + 48], xmm3",
    "    add rdx, 64",
    "    add rcx, 64",
    "    sub r8, 64",
    "    jmp victory_safe_copy_nt_64",
    "victory_safe_copy_nt_tail:",
    "    sfence",
    "    jmp victory_safe_copy",
    ".globl victory_safe_copy_end",
    "victory_safe_copy_end:",
    ".globl victory_safe_copy_fault",
    "victory_safe_copy_fault:",
    "    sfence",
    "    xor eax, eax",
    "    ret",
    // real addresses, not incremental-link thunks
    ".data",
    ".p2align 3",
    ".globl victory_safe_copy_bounds",
    "victory_safe_copy_bounds:",
    "    .quad victory_safe_copy",
    "    .quad victory_safe_copy_end",
    "    .quad victory_safe_copy_fault",
    ".text",
);

unsafe extern "system" {
    fn victory_safe_copy(dst: *mut u8, src: *const u8, len: usize) -> u32;
    fn victory_safe_copy_nt(dst: *mut u8, src: *const u8, len: usize) -> u32;
    static victory_safe_copy_bounds: [u64; 3];
}

/// title reads skip ReadProcessMemory (VICTORY_FAST_READ=1; off by default)
pub(crate) fn on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_FAST_READ").is_some_and(|v| v == "1"))
}

/// copies of 256 bytes and more store past the cache (VICTORY_COPY_STREAM=1; off by default)
pub(crate) fn stream() -> bool {
    // live: no state
    static S: crate::switch::Switch = crate::switch::Switch::noted("COPY_STREAM", "title reads store past the cache (VICTORY_COPY_STREAM=1)");
    S.on()
}

/// false if a page faulted; dst may hold the bytes before it, as with rpm
pub(crate) fn copy(dst: *mut u8, src: *const u8, len: usize) -> bool {
    copy_as(dst, src, len, stream())
}

fn copy_as(dst: *mut u8, src: *const u8, len: usize, streamed: bool) -> bool {
    if len == 0 {
        return true;
    }
    install();
    // SAFETY: a fault inside either loop resumes at the failure exit.
    unsafe {
        if streamed {
            victory_safe_copy_nt(dst, src, len) != 0
        } else {
            victory_safe_copy(dst, src, len) != 0
        }
    }
}

fn install() {
    use windows_sys::Win32::System::Diagnostics::Debug::{
        AddVectoredExceptionHandler, EXCEPTION_CONTINUE_EXECUTION, EXCEPTION_CONTINUE_SEARCH,
        EXCEPTION_POINTERS,
    };
    unsafe extern "system" fn on_fault(info: *mut EXCEPTION_POINTERS) -> i32 {
        // access violation, in-page error, guard page
        const CODES: [u32; 3] = [0xC000_0005, 0xC000_0006, 0x8000_0001];
        // SAFETY: the system passes the exception being dispatched.
        let (r, c) = unsafe { (&*(*info).ExceptionRecord, &mut *(*info).ContextRecord) };
        // SAFETY: written once by the loader's relocations.
        let [start, end, fault] = unsafe { victory_safe_copy_bounds };
        if !CODES.contains(&(r.ExceptionCode as u32)) || c.Rip < start || c.Rip >= end {
            return EXCEPTION_CONTINUE_SEARCH;
        }
        c.Rip = fault;
        EXCEPTION_CONTINUE_EXECUTION
    }
    static ONCE: std::sync::Once = std::sync::Once::new();
    // SAFETY: the handler only redirects faults raised inside our copy loop.
    ONCE.call_once(|| unsafe {
        AddVectoredExceptionHandler(1, Some(on_fault));
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn copies_and_reports_unmapped_sources() {
        use windows_sys::Win32::System::Memory::{
            MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_NOACCESS, PAGE_READWRITE, VirtualAlloc, VirtualFree,
        };
        let src: Vec<u8> = (0..1000u32).map(|i| i as u8).collect();
        for len in [0, 1, 15, 16, 63, 64, 65, 200, 1000] {
            let mut dst = vec![0u8; len];
            assert!(super::copy(dst.as_mut_ptr(), src.as_ptr(), len));
            assert_eq!(dst, src[..len]);
        }
        // SAFETY: a fresh reservation, freed below.
        let base = unsafe { VirtualAlloc(std::ptr::null(), 2 << 16, MEM_RESERVE, PAGE_NOACCESS) } as *mut u8;
        assert!(!base.is_null());
        // SAFETY: commits the first half of it.
        assert!(!unsafe { VirtualAlloc(base.cast_const().cast(), 1 << 16, MEM_COMMIT, PAGE_READWRITE) }.is_null());
        let mut dst = vec![0u8; 2 << 16];
        assert!(super::copy(dst.as_mut_ptr(), base, 1 << 16));
        assert!(!super::copy(dst.as_mut_ptr(), base, 2 << 16));
        // SAFETY: frees the reservation made above.
        unsafe { VirtualFree(base.cast(), 0, MEM_RELEASE) };
    }

    /// VICTORY_COPY_STREAM: the same bytes as the cached loop at every length
    /// and misalignment, and the same failure on an unmapped page.
    #[test]
    fn streamed_copies_match_cached_ones() {
        use windows_sys::Win32::System::Memory::{
            MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_NOACCESS, PAGE_READWRITE, VirtualAlloc, VirtualFree,
        };
        let src: Vec<u8> = (0..9000u32).map(|i| (i * 7 + 3) as u8).collect();
        for len in [0, 1, 63, 64, 255, 256, 257, 320, 1000, 4096, 4099, 8191] {
            for (s, d) in [(0, 0), (1, 0), (0, 1), (5, 11), (15, 3), (16, 16)] {
                let mut a = vec![0xa5u8; len + 64];
                let mut b = a.clone();
                assert!(super::copy_as(a[d..].as_mut_ptr(), src[s..].as_ptr(), len, false));
                assert!(super::copy_as(b[d..].as_mut_ptr(), src[s..].as_ptr(), len, true));
                assert_eq!(a, b, "len {len} src+{s} dst+{d}");
                assert_eq!(b[d..d + len], src[s..s + len]);
            }
        }
        // SAFETY: a fresh reservation, freed below.
        let base = unsafe { VirtualAlloc(std::ptr::null(), 2 << 16, MEM_RESERVE, PAGE_NOACCESS) } as *mut u8;
        assert!(!base.is_null());
        // SAFETY: commits the first half of it.
        assert!(!unsafe { VirtualAlloc(base.cast_const().cast(), 1 << 16, MEM_COMMIT, PAGE_READWRITE) }.is_null());
        let mut dst = vec![0u8; 2 << 16];
        assert!(super::copy_as(dst.as_mut_ptr(), base, 1 << 16, true));
        assert_eq!(dst[..1 << 16], vec![0u8; 1 << 16][..]);
        assert!(!super::copy_as(dst.as_mut_ptr(), base, 2 << 16, true));
        // SAFETY: frees the reservation made above.
        unsafe { VirtualFree(base.cast(), 0, MEM_RELEASE) };
    }

    /// offline numbers: GB/s of each loop on 64 KiB and 1 MiB runs out of a
    /// 512 MiB source (past the caches), and the cost of a 64-byte call
    #[test]
    #[ignore]
    fn bench_copy_loops() {
        const N: usize = 512 << 20;
        let src: Vec<u8> = (0..N).map(|i| i as u8).collect();
        let mut dst = vec![0u8; N];
        for run in [64usize << 10, 1 << 20] {
            for (name, how) in [("cached", 0), ("streamed", 1), ("ptr::copy", 2)] {
                let mut best = f64::MAX;
                for _ in 0..3 {
                    let t = std::time::Instant::now();
                    let mut at = 0;
                    while at + run <= N {
                        // SAFETY: inside both buffers.
                        unsafe {
                            match how {
                                0 => assert!(super::copy_as(dst.as_mut_ptr().add(at), src.as_ptr().add(at), run, false)),
                                1 => assert!(super::copy_as(dst.as_mut_ptr().add(at), src.as_ptr().add(at), run, true)),
                                _ => std::ptr::copy_nonoverlapping(src.as_ptr().add(at), dst.as_mut_ptr().add(at), run),
                            }
                        }
                        at += run;
                    }
                    best = best.min(t.elapsed().as_secs_f64());
                }
                println!("bench run {} KiB {name}: {:.2} GB/s", run >> 10, N as f64 / best / 1e9);
            }
        }
        let calls = 1_000_000;
        for (name, how) in [("guarded 64 B", 0), ("ptr::copy 64 B", 2)] {
            let t = std::time::Instant::now();
            for i in 0..calls {
                let at = (i * 4096) % (N - 64);
                // SAFETY: inside both buffers.
                unsafe {
                    if how == 0 {
                        super::copy_as(dst.as_mut_ptr().add(at), src.as_ptr().add(at), 64, false);
                    } else {
                        std::ptr::copy_nonoverlapping(src.as_ptr().add(at), dst.as_mut_ptr().add(at), 64);
                    }
                }
            }
            println!("bench {name}: {:.1} ns a call", t.elapsed().as_nanos() as f64 / calls as f64);
        }
        std::hint::black_box(&dst);
        // write-combined destination, as upload heaps map
        use windows_sys::Win32::System::Memory::{
            MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE, PAGE_WRITECOMBINE, VirtualAlloc, VirtualFree,
        };
        const W: usize = 256 << 20;
        // SAFETY: a fresh allocation, freed below.
        let wc = unsafe { VirtualAlloc(std::ptr::null(), W, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE | PAGE_WRITECOMBINE) } as *mut u8;
        assert!(!wc.is_null());
        for (name, streamed) in [("cached", false), ("streamed", true)] {
            let mut best = f64::MAX;
            for _ in 0..3 {
                let t = std::time::Instant::now();
                let mut at = 0;
                while at + (64 << 10) <= W {
                    // SAFETY: inside both buffers.
                    unsafe { assert!(super::copy_as(wc.add(at), src.as_ptr().add(at), 64 << 10, streamed)) };
                    at += 64 << 10;
                }
                best = best.min(t.elapsed().as_secs_f64());
            }
            println!("bench write-combined 64 KiB {name}: {:.2} GB/s", W as f64 / best / 1e9);
        }
        // SAFETY: our allocation.
        unsafe { VirtualFree(wc.cast(), 0, MEM_RELEASE) };
    }
}
