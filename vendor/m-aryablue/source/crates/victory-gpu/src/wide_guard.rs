//! A flush's guard system calls on the wide threads (`VICTORY_WIDE_GUARDS=1`
//! with `VICTORY_WIDE_COPIES=1`; off by default).
//!
//! Why: in the s2-prep-1 profile the submitting thread spent 70 ms a frame
//! in `watch::writes`, 20 of them making written view blocks read-only
//! again (`VirtualProtect`) and 16 asking Windows what memory is there
//! (`VirtualQuery`), one block run at a time; the kernel asking itself
//! (`GetWriteWatch`) was 1 ms.
//!
//! With the switch:
//! - before a flush's refreshes, the blocks noted absent in its ranges are
//!   asked about on the wide threads (each answer is where the uncommitted
//!   memory there ends, as the refresh would ask it a moment later);
//! - while it decides, a refresh notes the block runs it would make
//!   read-only instead of making them so;
//! - after its last refresh, under the guards' lock, the runs, merged where
//!   they meet, are made read-only and noted guarded in order, before the
//!   flush's bytes are moved (`crate::wide_copy`, which it needs: a block's
//!   bytes are taken only after it is read-only again, so a write in
//!   between is in them or faults).
//!
//! The read-only calls stay on this thread: they take the process's
//! address-space lock exclusively in the kernel, so more threads would
//! queue there, and a thread holding the guards' lock must not run other
//! wide work (a card merge writing a guarded block would fault into the
//! guards' handler and wait for that lock for good). The merging cuts their
//! number. The `VirtualQuery` askings take that lock shared, and run wide.
//!
//! The guards and the journal keep one writer, this thread, in order. A
//! block written again by a later refresh of the same flush, before its run
//! is read-only, is taken again: some more bytes, nothing missed.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

/// Whether a flush's guard calls go wide (needs [`crate::wide_copy::on`]).
pub(crate) fn on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        let on = std::env::var_os("VICTORY_WIDE_GUARDS").is_some_and(|v| v == "1")
            && crate::wide_copy::on();
        if on {
            tracing::info!("a flush's guard calls run on the wide threads (VICTORY_WIDE_GUARDS=1)");
        }
        on
    })
}

#[derive(Default)]
struct Flush {
    /// Block runs to make read-only.
    runs: Vec<(usize, usize)>,
    /// Uncommitted memory asked about ahead: start block to end block.
    ends: BTreeMap<usize, usize>,
}

thread_local! {
    static FLUSH: RefCell<Option<Flush>> = const { RefCell::new(None) };
}

/// Inside [`during`]: notes run `[from, to)` (blocks) to make read-only
/// after the refreshes; false outside, where the caller makes it so.
pub(crate) fn later(from: usize, to: usize) -> bool {
    FLUSH.with(|f| match f.borrow_mut().as_mut() {
        Some(flush) => {
            flush.runs.push((from, to));
            true
        }
        None => false,
    })
}

/// Inside [`during`]: where the uncommitted memory at `block` ends, if it
/// was asked ahead.
pub(crate) fn uncommitted_end_asked(block: usize) -> Option<usize> {
    FLUSH.with(|f| {
        let f = f.borrow();
        let (&start, &end) = f.as_ref()?.ends.range(..=block).next_back()?;
        (start <= block && block < end).then_some(end)
    })
}

/// Runs a flush's refreshes `f` over `ranges` (addresses) with the guard
/// calls as above; the runs are read-only when this returns.
pub(crate) fn during(ranges: &[(usize, usize)], f: impl FnOnce()) {
    struct Stop;
    impl Drop for Stop {
        fn drop(&mut self) {
            FLUSH.with(|f| *f.borrow_mut() = None);
        }
    }
    let early = crate::ask_early::on();
    // with commit notices the guard pass reads the answers on retry frames only
    let read = !crate::guard_log::commit_notice() || crate::guard_log::retry_frame(crate::frame::current());
    let ends = if !read {
        BTreeMap::new()
    } else if early {
        let (mut ends, left) = crate::ask_early::take(crate::watch::absent_in(ranges));
        ends.append(&mut ask(left));
        ends
    } else {
        ask_ahead(ranges)
    };
    FLUSH.with(|f| {
        *f.borrow_mut() = Some(Flush {
            runs: Vec::new(),
            ends,
        })
    });
    let stop = Stop;
    f();
    let runs = FLUSH
        .with(|f| f.borrow_mut().take())
        .map(|f| f.runs)
        .unwrap_or_default();
    drop(stop);
    let runs = merged(runs);
    if !runs.is_empty() {
        crate::watch::protect_runs(&runs, |runs| {
            runs.iter()
                .map(|&(a, b)| crate::watch::set_read_only(a, b))
                .collect()
        });
    }
    if early {
        crate::ask_early::next();
    }
}

/// `runs` sorted, those that meet or overlap made one.
pub(crate) fn merged(mut runs: Vec<(usize, usize)>) -> Vec<(usize, usize)> {
    runs.sort_unstable();
    let mut out: Vec<(usize, usize)> = Vec::with_capacity(runs.len());
    for (a, b) in runs {
        match out.last_mut() {
            Some(last) if a <= last.1 => last.1 = last.1.max(b),
            _ => out.push((a, b)),
        }
    }
    out
}

/// The absent blocks of `ranges` asked about on the wide threads: start
/// block to end block of each uncommitted stretch found.
fn ask_ahead(ranges: &[(usize, usize)]) -> BTreeMap<usize, usize> {
    ask(crate::watch::absent_in(ranges))
}

// `absent` (lists of blocks) asked about, on the wide threads if more than one list
fn ask(absent: Vec<Vec<usize>>) -> BTreeMap<usize, usize> {
    let found = Arc::new(Mutex::new(BTreeMap::new()));
    let walk = |blocks: &[usize], found: &Mutex<BTreeMap<usize, usize>>| {
        let mut until = 0;
        for &block in blocks {
            if block < until {
                continue;
            }
            until = crate::watch::uncommitted_end_of(block);
            if until > block {
                found
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(block, until);
            }
        }
    };
    if absent.len() < 2 {
        for blocks in &absent {
            walk(blocks, &found);
        }
    } else {
        let jobs: Vec<Box<dyn FnOnce() + Send>> = absent
            .into_iter()
            .map(|blocks| {
                let found = found.clone();
                Box::new(move || walk(&blocks, &found)) as Box<dyn FnOnce() + Send>
            })
            .collect();
        crate::shadow::wide_run(jobs);
    }
    std::mem::take(&mut *found.lock().unwrap_or_else(|e| e.into_inner()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_that_meet_are_made_one() {
        assert_eq!(
            merged(vec![(5, 7), (1, 3), (3, 4), (6, 9), (11, 12)]),
            vec![(1, 4), (5, 9), (11, 12)]
        );
    }

    #[test]
    fn runs_are_noted_only_during_a_flush() {
        assert!(!later(1, 2));
        let mut inside = (false, None);
        FLUSH.with(|f| {
            *f.borrow_mut() = Some(Flush {
                runs: Vec::new(),
                ends: BTreeMap::from([(10, 20)]),
            })
        });
        inside.0 = later(3, 4);
        inside.1 = uncommitted_end_asked(15);
        assert_eq!(uncommitted_end_asked(20), None);
        assert_eq!(uncommitted_end_asked(9), None);
        let runs = FLUSH.with(|f| f.borrow_mut().take()).unwrap().runs;
        assert_eq!(inside, (true, Some(20)));
        assert_eq!(runs, vec![(3, 4)]);
        assert!(!later(1, 2));
    }

    /// Runs made read-only are read-only, and only they (memory of our
    /// own).
    #[test]
    fn runs_set_read_only_are_read_only_and_only_they() {
        use windows_sys::Win32::System::Memory::{
            MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, MEMORY_BASIC_INFORMATION, PAGE_READONLY,
            PAGE_READWRITE, VirtualAlloc, VirtualFree, VirtualProtect, VirtualQuery,
        };
        const B: usize = 64 << 10;
        // SAFETY: fresh memory of our own, released below.
        let base = unsafe {
            VirtualAlloc(
                std::ptr::null(),
                24 * B,
                MEM_RESERVE | MEM_COMMIT,
                PAGE_READWRITE,
            )
        } as usize;
        assert_ne!(base, 0);
        let first = base.div_ceil(B);
        let runs: Vec<(usize, usize)> = (0..10)
            .map(|i| (first + 2 * i, first + 2 * i + 1))
            .collect();
        let answers: Vec<bool> = runs
            .iter()
            .map(|&(a, b)| crate::watch::set_read_only(a, b))
            .collect();
        assert!(answers.iter().all(|&a| a));
        for &(a, b) in &runs {
            let mut info: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
            // SAFETY: a valid out pointer of the right size.
            unsafe { VirtualQuery((a * B) as *const _, &mut info, size_of_val(&info)) };
            assert_eq!(
                (info.State, info.Protect),
                (MEM_COMMIT, PAGE_READONLY),
                "{a}..{b}"
            );
            let mut info: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
            // SAFETY: as above.
            unsafe { VirtualQuery((b * B) as *const _, &mut info, size_of_val(&info)) };
            assert_eq!(info.Protect, PAGE_READWRITE, "the block after {a}..{b}");
        }
        let mut old = 0;
        // SAFETY: our memory.
        unsafe {
            VirtualProtect(base as *const _, 24 * B, PAGE_READWRITE, &mut old);
            VirtualFree(base as *mut _, 0, MEM_RELEASE);
        }
    }
}
