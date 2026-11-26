//! What the guards' fault handler learns, kept for the dirty pass instead
//! of found again by walking every guarded block (F1's P2).
//!
//! Measured on dlss-s-1: the handler learns every written block (347 a
//! frame), yet each frame the pass walked ~125,000 blocks of the clean set
//! to find them again, nine checks in ten on view memory, which the
//! kernel's write watch cannot see.
//!
//! - `VICTORY_FAULT_LOG=1`: the guards keep, beside the clean set, the set
//!   of blocks that are not clean, valid inside the stretches a full pass
//!   has once looked through ([`Covered`]). The handler's opening puts a
//!   block there; a pass over a covered range reads only those blocks.
//! - `VICTORY_HOT_OPEN=1`: a block written in at least 3 of the last 4
//!   frames is left open after its copy takes it, and counts as written at
//!   every pass (not one with other views: a write through it would not be
//!   seen at theirs); closed again every [`RETRY`] frames.
//!   `VICTORY_HOT_COUNT=1` (or the switch) counts such blocks and their
//!   bytes into the log every 256 frames.
//! - `VICTORY_COMMIT_NOTICE=1`: blocks noted absent (uncommitted) are not
//!   asked about again at every pass; the title's commits through XMem say
//!   when they arrive (`title_memory_committed`), and every [`RETRY`]
//!   frames they are asked about anyway, for a commit that went round it.

use std::collections::BTreeMap;

fn switch(name: &str) -> bool {
    std::env::var_os(name).is_some_and(|v| v == "1")
}

/// On in the tests: every guard test runs its later checks from the log.
pub(crate) fn fault_log() -> bool {
    // live: switched off, the next pass forgets the covered stretches (`watch::guard`)
    static S: crate::switch::Switch = crate::switch::Switch::new("FAULT_LOG");
    cfg!(test) || S.on()
}

pub(crate) fn hot_open() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| switch("VICTORY_HOT_OPEN"))
}

pub(crate) fn hot_count() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| hot_open() || switch("VICTORY_HOT_COUNT"))
}

pub(crate) fn commit_notice() -> bool {
    // live: commits clear absent blocks with the switch off too
    static S: crate::switch::Switch = crate::switch::Switch::new("COMMIT_NOTICE");
    S.on()
}

/// Since the last frame line, with any of these switches on: what the
/// guard passes did (see [`guard_line`]).
pub(crate) static COUNTS: [std::sync::atomic::AtomicU64; 13] =
    [const { std::sync::atomic::AtomicU64::new(0) }; 13];
/// blocks read from the log (a covered range's blocks not clean)
pub(crate) const FROM_LOG: usize = 0;
/// blocks walked the old way (ranges not covered)
pub(crate) const WALKED: usize = 1;
pub(crate) const RANGES_COVERED: usize = 2;
/// not covered: no part of the range looked through yet
pub(crate) const RANGES_NEW: usize = 3;
/// not covered: partly looked through (new blocks at an end, or a remap cut it)
pub(crate) const RANGES_PART: usize = 4;
/// mapped or unmapped memory that cut covered stretches
pub(crate) const REMAP_CUTS: usize = 5;
pub(crate) const QUERIES: usize = 6;
/// absent blocks not asked about (commit notices)
pub(crate) const ASK_SKIPPED: usize = 7;
/// absent blocks the title's commits made due again
pub(crate) const COMMITS_NOTICED: usize = 8;
/// absent blocks asked about on retry frames
pub(crate) const NET_ASKED: usize = 9;
/// of those, found committed: commits the notices missed
pub(crate) const NET_FOUND: usize = 10;
/// hot blocks left open at passes
pub(crate) const HOT: usize = 11;
pub(crate) const NET_PASSES: usize = 12;

pub(crate) fn count(i: usize, n: usize) {
    COUNTS[i].fetch_add(n as u64, std::sync::atomic::Ordering::Relaxed);
}

/// Whether the frame line says what the guard passes did.
pub(crate) fn any_on() -> bool {
    fault_log() || commit_notice() || hot_count()
}

/// With the frame line: [`COUNTS`] per frame over `frames`, and what the
/// guards hold now (`state`: blocks not clean in the log, covered
/// stretches, absent blocks).
pub(crate) fn guard_line(frames: f64, block_bytes: usize, state: (usize, usize, usize)) {
    let n = COUNTS
        .each_ref()
        .map(|c| c.swap(0, std::sync::atomic::Ordering::Relaxed) as f64 / frames);
    tracing::info!(
        "guard passes per frame (VICTORY_FAULT_LOG {}, VICTORY_COMMIT_NOTICE {}, VICTORY_HOT_OPEN {}): blocks read \
         from the log {:.0}, walked the old way {:.0}; ranges covered {:.1}, not covered {:.1} (none looked through \
         yet {:.1}, partly {:.1}), remaps that cut stretches {:.1}; VirtualQuery made {:.1}, absent blocks not asked \
         {:.0}, commits noticed {:.1} blocks, safety-net passes {:.2} asking {:.0} blocks, {:.1} found committed; hot \
         blocks open {:.0} ({:.1} MB); now {} blocks not clean in the log, {} covered stretches, {} absent",
        fault_log() as u8,
        commit_notice() as u8,
        hot_open() as u8,
        n[FROM_LOG],
        n[WALKED],
        n[RANGES_COVERED],
        n[RANGES_NEW] + n[RANGES_PART],
        n[RANGES_NEW],
        n[RANGES_PART],
        n[REMAP_CUTS],
        n[QUERIES],
        n[ASK_SKIPPED],
        n[COMMITS_NOTICED],
        n[NET_PASSES],
        n[NET_ASKED],
        n[NET_FOUND],
        n[HOT],
        n[HOT] * block_bytes as f64 / (1 << 20) as f64,
        state.0,
        state.1,
        state.2
    );
}

/// Frames between a hot block's closings, and between askings about absent
/// blocks with commit notices.
pub(crate) const RETRY: u64 = 64;

/// Whether `frame` is one where open hot blocks close and absent blocks
/// are asked about.
pub(crate) fn retry_frame(frame: u64) -> bool {
    frame % RETRY == 0
}

/// Stretches of blocks `[start, end)` a full pass has looked through,
/// apart and in order.
#[derive(Default)]
pub(crate) struct Covered(BTreeMap<usize, usize>);

impl Covered {
    pub(crate) const fn new() -> Self {
        Covered(BTreeMap::new())
    }

    /// Whether `[a, b)` lies inside one stretch.
    pub(crate) fn holds(&self, a: usize, b: usize) -> bool {
        a >= b
            || self
                .0
                .range(..=a)
                .next_back()
                .is_some_and(|(_, &end)| end >= b)
    }

    pub(crate) fn add(&mut self, mut a: usize, mut b: usize) {
        if a >= b {
            return;
        }
        // the stretches that meet [a, b) are made one with it
        if let Some((&s, &e)) = self.0.range(..=a).next_back()
            && e >= a
        {
            a = s;
            b = b.max(e);
        }
        let inside: Vec<(usize, usize)> = self.0.range(a..=b).map(|(&s, &e)| (s, e)).collect();
        for (s, e) in inside {
            self.0.remove(&s);
            b = b.max(e);
        }
        self.0.insert(a, b);
    }

    pub(crate) fn remove(&mut self, a: usize, b: usize) {
        if a >= b {
            return;
        }
        let mut hit: Vec<(usize, usize)> = self.0.range(a..b).map(|(&s, &e)| (s, e)).collect();
        if let Some((&s, &e)) = self.0.range(..a).next_back()
            && e > a
        {
            hit.push((s, e));
        }
        for (s, e) in hit {
            self.0.remove(&s);
            if s < a {
                self.0.insert(s, a);
            }
            if e > b {
                self.0.insert(b, e);
            }
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub(crate) fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether any stretch meets `[a, b)`.
    pub(crate) fn touches(&self, a: usize, b: usize) -> bool {
        a < b
            && (self.0.range(a..b).next().is_some()
                || self.0.range(..a).next_back().is_some_and(|(_, &e)| e > a))
    }
}

/// Per block, the frames it was written in lately: the last such frame and
/// a mask of it and the 7 before (bit k: frame `last - k`).
#[derive(Default)]
pub(crate) struct Faults {
    seen: BTreeMap<usize, (u64, u8)>,
    said: u64,
}

impl Faults {
    pub(crate) const fn new() -> Self {
        Faults {
            seen: BTreeMap::new(),
            said: 0,
        }
    }

    pub(crate) fn note(&mut self, block: usize, frame: u64) {
        let e = self.seen.entry(block).or_insert((frame, 0));
        e.1 = shifted(e.1, frame.saturating_sub(e.0)) | 1;
        e.0 = e.0.max(frame);
    }

    /// Written in at least 3 of the 4 frames up to `frame`.
    pub(crate) fn hot(&self, block: usize, frame: u64) -> bool {
        self.seen.get(&block).is_some_and(|&(last, mask)| {
            (shifted(mask, frame.saturating_sub(last)) & 0b1111).count_ones() >= 3
        })
    }

    /// Every 256 frames: the hot blocks and their bytes, said in the log;
    /// blocks not written for 8 frames are forgotten.
    pub(crate) fn census(&mut self, frame: u64, block_bytes: usize) {
        if frame < self.said + 256 {
            return;
        }
        self.said = frame;
        self.seen
            .retain(|_, &mut (last, _)| frame.saturating_sub(last) < 8);
        let hot = self.seen.keys().filter(|&&b| self.hot(b, frame)).count();
        tracing::info!(
            hot,
            mb = (hot * block_bytes) as f64 / (1 << 20) as f64,
            written_lately = self.seen.len(),
            "guarded blocks written in 3 of the last 4 frames (VICTORY_HOT_COUNT)"
        );
    }
}

fn shifted(mask: u8, by: u64) -> u8 {
    if by >= 8 { 0 } else { mask << by }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn covered_stretches_join_and_split() {
        let mut c = Covered::default();
        c.add(10, 20);
        c.add(30, 40);
        assert!(c.holds(12, 18) && c.holds(10, 20) && !c.holds(15, 25) && !c.holds(20, 31));
        c.add(20, 30);
        assert!(c.holds(10, 40));
        assert_eq!(c.0.len(), 1);
        c.remove(15, 17);
        assert!(c.holds(10, 15) && c.holds(17, 40) && !c.holds(14, 16) && !c.holds(16, 17));
        assert!(c.touches(0, 11) && c.touches(39, 60) && !c.touches(40, 60) && !c.touches(0, 10));
        c.remove(0, 100);
        assert!(c.is_empty());
        c.add(5, 8);
        c.add(1, 50);
        assert_eq!(c.0.len(), 1);
        assert!(c.holds(1, 50));
    }

    #[test]
    fn a_block_is_hot_after_3_of_4_frames() {
        let mut f = Faults::default();
        f.note(7, 100);
        f.note(7, 101);
        assert!(!f.hot(7, 101));
        f.note(7, 103);
        assert!(f.hot(7, 103));
        // at 104 frame 100 is out: 101 and 103 left
        assert!(!f.hot(7, 104));
        f.note(7, 104);
        assert!(f.hot(7, 104));
        assert!(!f.hot(8, 104));
    }
}
