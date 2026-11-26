//! Refresh work that follows what changed, not what is named
//! (`VICTORY_PUSH_DIRTY`, off by default).
//!
//! Level 1: every change to title memory we come to know of counts at its
//! megabyte ([`changed`]); a copy remembers the ranges its last refreshes
//! found nothing to copy in, with the counts as they were, and a refresh of
//! such a range is over when the counts still stand.
//! `VICTORY_PUSH_CHECK=1` decides the old way as well and says where the two
//! differ, by bytes.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

use crate::watch::PAGE;

/// The level asked for: 0 off.
pub(crate) fn level() -> u8 {
    static ON: OnceLock<u8> = OnceLock::new();
    *ON.get_or_init(|| {
        let level = std::env::var("VICTORY_PUSH_DIRTY").ok().and_then(|v| v.parse::<u8>().ok()).unwrap_or(0);
        if level > 0 {
            tracing::info!(
                level,
                check = check(),
                "a refresh of a range nothing was written in since its last one is over at once (VICTORY_PUSH_DIRTY)"
            );
        }
        level
    })
}

// on in the crate's tests, so that they run through it
pub(crate) fn on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| level() > 0 || (cfg!(test) && std::env::var_os("VICTORY_PUSH_DIRTY").is_none()))
}

/// `VICTORY_PUSH_CHECK=1`: the old decision is made too, and stands.
pub(crate) fn check() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_PUSH_CHECK").is_some_and(|v| v == "1"))
}

// pages to a region, and the regions counted apart
const REGION: usize = 16;
const SLOTS: usize = 1 << 16;
// a change over more regions than this counts once for everything
const WIDE: usize = 4096;

static COUNTS: [AtomicU64; SLOTS] = [const { AtomicU64::new(0) }; SLOTS];
static EVERYWHERE: AtomicU64 = AtomicU64::new(0);

/// The 64 KiB page numbered `page` changed, or may have: written by the
/// title (found by the write watch or opened by the guards), by us, mapped,
/// unmapped or committed.
pub(crate) fn changed(page: usize) {
    COUNTS[(page / REGION) % SLOTS].fetch_add(1, Relaxed);
}

/// [`changed`] for pages `[first, last)`.
pub(crate) fn changed_pages(first: usize, last: usize) {
    if first >= last {
        return;
    }
    let (a, b) = (first / REGION, (last - 1) / REGION);
    if b - a >= WIDE {
        EVERYWHERE.fetch_add(1, Relaxed);
        return;
    }
    for region in a..=b {
        COUNTS[region % SLOTS].fetch_add(1, Relaxed);
    }
}

/// The counts over `[from, to)` (addresses), as one number: it differs
/// after any change inside (counts only grow).
pub(crate) fn token(from: usize, to: usize) -> u64 {
    let mut sum = EVERYWHERE.load(Relaxed);
    if from >= to {
        return sum;
    }
    let (a, b) = (from / PAGE / REGION, (to - 1) / PAGE / REGION);
    if b - a >= SLOTS {
        for count in &COUNTS {
            sum = sum.wrapping_add(count.load(Relaxed));
        }
        return sum;
    }
    for region in a..=b {
        sum = sum.wrapping_add(COUNTS[region % SLOTS].load(Relaxed));
    }
    sum
}

/// Ranges of a copy that held the title's bytes when last refreshed, each
/// with its [`token`] from before that refresh asked.
#[derive(Default)]
pub(crate) struct Quiet {
    ranges: [(usize, usize, u64); Quiet::MOST],
    next: usize,
}

impl Quiet {
    const MOST: usize = 4;

    /// Whether `[from, to)` lies in a range noted with nothing changed since.
    pub(crate) fn holds(&self, from: usize, to: usize) -> Option<bool> {
        let mut known = None;
        for &(a, b, at) in &self.ranges {
            if a <= from && to <= b && a < b {
                if at == token(a, b) {
                    return Some(true);
                }
                known = Some(false);
            }
        }
        known
    }

    /// `[from, to)` had nothing to copy; `at` is its token from before the asking.
    pub(crate) fn note(&mut self, from: usize, to: usize, at: u64) {
        let slot = self.ranges.iter().position(|&(a, b, _)| a < to && from < b).unwrap_or_else(|| {
            self.next = (self.next + 1) % Self::MOST;
            self.next
        });
        self.ranges[slot] = (from, to, at);
    }
}

/// Steps over at once; steps with no range noted; with one whose counts
/// moved; with one the journal or the guards would have to ask for; checked
/// steps; of those the old way copied into, with the same bytes; with other
/// bytes; too long to compare; the bytes that differed; frames.
static STATS: [AtomicU64; 10] = [const { AtomicU64::new(0) }; 10];

pub(crate) fn stat(i: usize, n: u64) {
    STATS[i].fetch_add(n, Relaxed);
}

/// A checked step the old way copied `copied` bytes for; `differing` is
/// what [`crate::shadow::Shadow::differs`] said before it did.
pub(crate) fn missed(address: usize, len: usize, copied: usize, differing: Option<usize>) {
    let class = match differing {
        Some(0) => 5,
        Some(n) => {
            stat(8, n as u64);
            6
        }
        None => 7,
    };
    stat(class, 1);
    static SAID: AtomicU64 = AtomicU64::new(0);
    if class != 5 && SAID.fetch_add(1, Relaxed) < 32 {
        tracing::warn!(
            address = format_args!("{address:#x}"),
            len,
            copied,
            differing = ?differing,
            "push check: a refresh held to be over had bytes to copy"
        );
    }
}

// at a refresh: counts the frames
pub(crate) fn tick() {
    static LAST: AtomicU64 = AtomicU64::new(0);
    let now = crate::frame::current();
    if LAST.swap(now, Relaxed) != now {
        frame();
    }
}

fn frame() {
    const EVERY: u64 = 256;
    if STATS[9].fetch_add(1, Relaxed) + 1 < EVERY {
        return;
    }
    let s: [u64; 10] = std::array::from_fn(|i| STATS[i].swap(0, Relaxed));
    let per = |n: u64| n as f64 / s[9].max(1) as f64;
    tracing::info!(
        over = format_args!("{:.0}", per(s[0])),
        unnoted = format_args!("{:.0}", per(s[1])),
        moved = format_args!("{:.0}", per(s[2])),
        to_ask = format_args!("{:.0}", per(s[3])),
        checked = s[4],
        same_bytes = s[5],
        other_bytes = s[6],
        long = s[7],
        differing = s[8],
        "push dirty, a frame: refreshes over at once, with no range noted, changed since, to be asked about; the check's steps and misses over {EVERY} frames"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    const MB: usize = 1 << 20;

    #[test]
    fn a_change_moves_the_tokens_over_it_and_no_others() {
        let base = 0x7a_0000_0000usize;
        let (near, far) = (token(base, base + 4 * MB), token(base + 8 * MB, base + 9 * MB));
        changed((base + 2 * MB) / PAGE);
        assert_ne!(token(base, base + 4 * MB), near);
        assert_eq!(token(base + 8 * MB, base + 9 * MB), far);
        let near = token(base, base + 4 * MB);
        changed_pages((base + 3 * MB) / PAGE, (base + 12 * MB) / PAGE);
        assert_ne!(token(base, base + 4 * MB), near);
        assert_ne!(token(base + 8 * MB, base + 9 * MB), far);
        // the last page of a range counts, the one after does not
        let at = token(base + 20 * MB, base + 21 * MB);
        changed((base + 21 * MB) / PAGE);
        assert_eq!(token(base + 20 * MB, base + 21 * MB), at);
        changed((base + 21 * MB) / PAGE - 1);
        assert_ne!(token(base + 20 * MB, base + 21 * MB), at);
    }

    #[test]
    fn a_wide_change_moves_every_token() {
        let base = 0x7b_0000_0000usize;
        let at = token(base, base + MB);
        changed_pages(0x10, 0x10 + WIDE * REGION + 1);
        assert_ne!(token(base, base + MB), at);
    }

    #[test]
    fn a_noted_range_holds_until_a_change_or_a_copy() {
        let base = 0x7c_0000_0000usize;
        let mut quiet = Quiet::default();
        assert_eq!(quiet.holds(base, base + MB), None);
        quiet.note(base, base + 2 * MB, token(base, base + 2 * MB));
        assert_eq!(quiet.holds(base, base + MB), Some(true));
        assert_eq!(quiet.holds(base + MB, base + 2 * MB), Some(true));
        // a range reaching past it is not held
        assert_eq!(quiet.holds(base + MB, base + 3 * MB), None);
        changed((base + MB) / PAGE);
        assert_eq!(quiet.holds(base, base + MB), Some(false));
        // noted before the change was asked about: stays moved
        let before = token(base, base + 2 * MB);
        changed(base / PAGE);
        quiet.note(base, base + 2 * MB, before);
        assert_eq!(quiet.holds(base, base + MB), Some(false));
        quiet.note(base, base + 2 * MB, token(base, base + 2 * MB));
        assert_eq!(quiet.holds(base, base + MB), Some(true));
    }

    #[test]
    fn ranges_of_one_copy_are_noted_side_by_side() {
        let base = 0x7d_0000_0000usize;
        let mut quiet = Quiet::default();
        for i in 0..Quiet::MOST {
            let (a, b) = (base + i * 4 * MB, base + i * 4 * MB + MB);
            quiet.note(a, b, token(a, b));
        }
        for i in 0..Quiet::MOST {
            assert_eq!(quiet.holds(base + i * 4 * MB, base + i * 4 * MB + MB), Some(true));
        }
        // one more takes the oldest slot's place, the rest stay
        let far = base + 64 * MB;
        quiet.note(far, far + MB, token(far, far + MB));
        assert_eq!(quiet.holds(far, far + MB), Some(true));
        let held = (0..Quiet::MOST).filter(|i| quiet.holds(base + i * 4 * MB, base + i * 4 * MB + MB) == Some(true)).count();
        assert_eq!(held, Quiet::MOST - 1);
    }
}
