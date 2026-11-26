//! Copies past unmapped title memory a region at a time
//! (`VICTORY_SKIP_UNMAPPED=0` turns it off): a failed 4 KiB read asks the
//! kernel for the page's region and zeros all of it that is not readable,
//! instead of failing one read per page (~4 µs each, ~16k per world frame).

pub(crate) fn on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("VICTORY_SKIP_UNMAPPED").map_or(true, |v| v != "0"))
}

/// Where the unreadable memory at `at` ends, if it is unreadable as a
/// whole region (reserved, free, no access or guarded).
fn unreadable_until(at: usize) -> Option<usize> {
    use windows_sys::Win32::System::Memory::{
        MEM_COMMIT, MEMORY_BASIC_INFORMATION, PAGE_GUARD, PAGE_NOACCESS, VirtualQuery,
    };
    #[cfg(feature = "diagnostics")]
    let started = std::time::Instant::now();
    // SAFETY: plain data filled in by the query.
    let mut info: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: a valid out pointer of the right size.
    let n = unsafe { VirtualQuery(at as *const _, &mut info, size_of_val(&info)) };
    #[cfg(feature = "diagnostics")]
    {
        use crate::copystat::{C, add};
        add(C::Queries, 1);
        add(C::QueryUs, started.elapsed().as_micros() as u64);
    }
    if n == 0 {
        return None;
    }
    let unreadable =
        info.State != MEM_COMMIT || info.Protect & (PAGE_NOACCESS | PAGE_GUARD) != 0;
    unreadable.then(|| info.BaseAddress as usize + info.RegionSize)
}

/// The fallback of [`crate::host::copy_mapped`]: `[src, src + len)` into
/// `dst` with zeros for what cannot be read; `read` copies in the kernel.
pub(crate) fn copy(src: usize, dst: *mut u8, len: usize, read: impl Fn(usize, *mut u8, usize) -> bool) {
    const BIG: usize = 64 << 10;
    const SMALL: usize = 4 << 10;
    let end = src + len;
    let to = |at: usize| dst.wrapping_add(at - src);
    let zero = |a: usize, b: usize| {
        #[cfg(feature = "diagnostics")]
        crate::copystat::add(crate::copystat::C::Unmapped, (b - a) as u64);
        // SAFETY: inside the destination, which is mapped.
        unsafe { to(a).write_bytes(0, b - a) };
    };
    let mut at = src;
    'big: while at < end {
        let next = ((at / BIG + 1) * BIG).min(end);
        if read(at, to(at), next - at) {
            at = next;
            continue;
        }
        while at < next {
            let small = ((at / SMALL + 1) * SMALL).min(next);
            if read(at, to(at), small - at) {
                at = small;
                continue;
            }
            match unreadable_until(at) {
                Some(until) if until > small => {
                    let until = until.min(end);
                    zero(at, until);
                    at = until;
                    continue 'big;
                }
                _ => {
                    zero(at, small);
                    at = small;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use windows_sys::Win32::System::Memory::{
        MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE, VirtualAlloc, VirtualFree,
    };

    /// A run of unmapped blocks costs one failed read and a query, not a
    /// failed read per page, and reads as zeros between mapped bytes.
    #[test]
    fn unmapped_runs_are_skipped_whole() {
        const BLOCK: usize = 64 << 10;
        // SAFETY: a fresh reservation; blocks 0 and 9 committed.
        let base = unsafe { VirtualAlloc(std::ptr::null(), 10 * BLOCK, MEM_RESERVE, PAGE_READWRITE) } as usize;
        assert_ne!(base, 0);
        for b in [0, 9] {
            // SAFETY: inside the reservation.
            let p = unsafe { VirtualAlloc((base + b * BLOCK) as *const _, BLOCK, MEM_COMMIT, PAGE_READWRITE) };
            assert!(!p.is_null());
            // SAFETY: committed just now.
            unsafe { (p as *mut u8).write_bytes(7, BLOCK) };
        }
        let mut out = vec![0xeeu8; 10 * BLOCK];
        let failed = Cell::new(0);
        let read = |at: usize, to: *mut u8, n: usize| {
            let ok = crate::host::read_into(at as *const u8, unsafe { std::slice::from_raw_parts_mut(to, n) });
            failed.set(failed.get() + usize::from(!ok));
            ok
        };
        super::copy(base, out.as_mut_ptr(), out.len(), read);
        assert!(out[..BLOCK].iter().all(|&b| b == 7));
        assert!(out[BLOCK..9 * BLOCK].iter().all(|&b| b == 0));
        assert!(out[9 * BLOCK..].iter().all(|&b| b == 7));
        // the old page-by-page fallback failed 8 * 17 reads here
        assert!(failed.get() <= 2, "{} failed reads", failed.get());
        // SAFETY: our reservation.
        unsafe { VirtualFree(base as *mut _, 0, MEM_RELEASE) };
    }
}
