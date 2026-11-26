//! Indirect clamps batched ahead per segment from what the same segment read last time (`VICTORY_CLAMP_BATCH=1`, needs `VICTORY_WINDOWS_MANY=1`).

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

/// Whether segments' clamps are batched.
pub(crate) fn on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_CLAMP_BATCH").is_some_and(|v| v == "1"))
}

/// Since the frame line: batches clamped, windows clamped in them past their head, plans
/// kept, windows of plans that no draw read, windows of plans a draw read, windows
/// clamped one by one in a segment past its head, heads whose plan rests.
pub(crate) static STATS: [AtomicU64; 7] = [const { AtomicU64::new(0) }; 7];

/// A window of a plan: its first record's offset from the head's, the step between its
/// records and how many.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Planned {
    pub(crate) delta: i64,
    pub(crate) step: u32,
    pub(crate) records: u32,
}

impl Planned {
    /// Whether it holds the record at `delta` from the head.
    fn holds(&self, delta: i64) -> bool {
        delta >= self.delta
            && (delta - self.delta) % i64::from(self.step.max(1)) == 0
            && (delta - self.delta) / i64::from(self.step.max(1)) < i64::from(self.records)
    }
}

/// A segment's head: pipeline, kind, stride, and the how-manieth head of those in the list.
pub(crate) type Key = (usize, u32, u32, u32);

/// The segment a list is in, and how many heads of each kind it saw.
#[derive(Default)]
pub(crate) struct Segment {
    /// The head's key, its argument copy and its record's offset there.
    head: Option<(Key, u64, u64)>,
    /// Windows clamped after the head: whether a draw read each, whether it was planned.
    seen: Vec<(Planned, bool, bool)>,
    nth: HashMap<(usize, u32, u32), u32>,
}

impl Segment {
    /// A clamp with no segment open begins one: its key.
    pub(crate) fn begin(&mut self, (pipeline, kind, stride): (usize, u32, u32), copy: u64, offset: u64) -> Key {
        let n = self.nth.entry((pipeline, kind, stride)).or_insert(0);
        *n += 1;
        let key = (pipeline, kind, stride, *n);
        self.head = Some((key, copy, offset));
        self.seen.clear();
        key
    }

    pub(crate) fn open(&self) -> bool {
        self.head.is_some()
    }

    /// The offset from the head's record of the record at `offset` in `copy`, if in its copy.
    fn delta(&self, copy: u64, offset: u64) -> Option<i64> {
        let &(_, head_copy, head) = self.head.as_ref()?;
        (copy == head_copy).then(|| offset as i64 - head as i64)
    }

    /// A window clamped in the segment past its head: `read`, whether a draw reads it now
    /// (one clamped alone), else it came from the plan.
    pub(crate) fn note(&mut self, copy: u64, offset: u64, step: u32, records: u32, read: bool) {
        if let Some(delta) = self.delta(copy, offset)
            && delta != 0
        {
            self.seen.push((Planned { delta, step, records }, read, !read));
        }
    }

    /// A draw read the record at `offset` of `copy` from a window: whether that was the
    /// first read of a planned window.
    pub(crate) fn read(&mut self, copy: u64, offset: u64) -> bool {
        if let Some(delta) = self.delta(copy, offset)
            && let Some(w) = self.seen.iter_mut().rev().find(|(p, _, _)| p.holds(delta))
        {
            let first = w.2 && !w.1;
            w.1 = true;
            return first;
        }
        false
    }

    /// The segment ends: its head and the windows clamped past it (read, planned), if any.
    pub(crate) fn end(&mut self) -> Option<(Key, Vec<(Planned, bool, bool)>)> {
        let (key, _, _) = self.head.take()?;
        Some((key, std::mem::take(&mut self.seen)))
    }

    /// Another host list: heads are counted afresh.
    pub(crate) fn new_list(&mut self) {
        self.head = None;
        self.seen.clear();
        self.nth.clear();
    }
}

/// Most plans kept; past it they are all forgotten.
const PLANS_MOST: usize = 8192;

/// Most windows a head keeps track of.
const WINDOWS_MOST: usize = 128;

/// Times a head comes round a window is remembered unread (the low bits of its history).
const REMEMBERED: u8 = 0b1111;

/// Planned only when a draw read it each of the last two times.
const PLANNED: u8 = 0b11;

/// Planned windows looked at before a head's hit rate is judged, and how many times
/// a head with fewer than half read rests with no plan.
const JUDGED: u32 = 32;
const REST: u8 = 16;

/// What a head remembers: its windows with their history (bit 0: read the last time),
/// its planned windows and how many of them a draw read, and times left to rest.
#[derive(Default)]
struct Kept {
    windows: Vec<(Planned, u8)>,
    planned: u32,
    read: u32,
    rest: u8,
}

impl Kept {
    fn plan(&self) -> Vec<Planned> {
        if self.rest > 0 {
            return Vec::new();
        }
        self.windows.iter().filter(|(_, h)| h & PLANNED == PLANNED).map(|(p, _)| *p).collect()
    }

    /// The head came round again: what was clamped past it and read.
    fn round(&mut self, seen: &[(Planned, bool, bool)]) {
        let planned = seen.iter().filter(|w| w.2).count() as u32;
        let read = seen.iter().filter(|w| w.2 && w.1).count() as u32;
        self.planned += planned;
        self.read += read;
        if self.planned >= JUDGED {
            if self.read * 2 < self.planned {
                self.rest = REST;
            }
            self.planned = 0;
            self.read = 0;
        } else if self.rest > 0 && planned == 0 {
            self.rest -= 1;
        }
        for w in &mut self.windows {
            w.1 <<= 1;
        }
        for &(p, was_read, _) in seen {
            match self.windows.iter_mut().find(|(q, _)| *q == p) {
                Some(w) => w.1 |= u8::from(was_read),
                None if was_read => self.windows.push((p, 1)),
                None => {}
            }
        }
        self.windows.retain(|(_, h)| h & REMEMBERED != 0);
        if self.windows.len() > WINDOWS_MOST {
            let over = self.windows.len() - WINDOWS_MOST;
            self.windows.drain(..over);
        }
    }
}

static PLANS: Mutex<Option<HashMap<Key, Kept>>> = Mutex::new(None);

/// The plan for a head: the windows a draw read each of the last two times it came round.
pub(crate) fn plan(key: Key) -> Option<Vec<Planned>> {
    let plans = PLANS.lock().unwrap_or_else(|e| e.into_inner());
    let kept = plans.as_ref()?.get(&key)?;
    if kept.rest > 0 {
        STATS[6].fetch_add(1, Ordering::Relaxed);
    }
    Some(kept.plan()).filter(|p| !p.is_empty())
}

/// A head's segment ended: what was clamped past it goes into what the head remembers.
pub(crate) fn keep(key: Key, seen: &[(Planned, bool, bool)]) {
    for &(_, read, planned) in seen {
        let at = match (planned, read) {
            (true, false) => 3,
            (true, true) => 4,
            (false, _) => 5,
        };
        STATS[at].fetch_add(1, Ordering::Relaxed);
    }
    let mut plans = PLANS.lock().unwrap_or_else(|e| e.into_inner());
    let map = plans.get_or_insert_with(HashMap::new);
    if map.len() >= PLANS_MOST && !map.contains_key(&key) {
        map.clear();
    }
    let kept = map.entry(key).or_default();
    kept.round(seen);
    if kept.windows.is_empty() && kept.rest == 0 {
        map.remove(&key);
    } else if !kept.plan().is_empty() {
        STATS[2].fetch_add(1, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::{Kept, Planned, Segment};

    const A: Planned = Planned { delta: 0x800, step: 32, records: 16 };
    const B: Planned = Planned { delta: 0x2000, step: 32, records: 16 };

    #[test]
    fn a_segment_hands_over_what_was_clamped_past_its_head() {
        let mut s = Segment::default();
        let key = s.begin((7, 10, 12), 1, 0x100);
        assert_eq!(key, (7, 10, 12, 1));
        // one clamped alone for its draw, one from the plan no draw read; one in another copy
        s.note(1, 0x900, 32, 16, true);
        s.note(1, 0x2100, 32, 16, false);
        s.note(2, 0x100, 32, 16, true);
        // the head itself is not part of it
        s.note(1, 0x100, 32, 16, true);
        let (k, seen) = s.end().expect("a segment");
        assert_eq!(k, key);
        assert_eq!(seen, [(A, true, false), (B, false, true)]);
        assert!(!s.open());
        // the same head again in the list is the second of its kind
        assert_eq!(s.begin((7, 10, 12), 1, 0x100).3, 2);
        s.new_list();
        assert_eq!(s.begin((7, 10, 12), 1, 0x100).3, 1);
    }

    #[test]
    fn a_planned_window_is_marked_read_by_a_draw_of_any_of_its_records() {
        let mut s = Segment::default();
        s.begin((7, 10, 12), 1, 0x100);
        s.note(1, 0x1100, 32, 4, false);
        // not on the step, past the end
        s.read(1, 0x1110);
        s.read(1, 0x1100 + 4 * 32);
        assert_eq!(s.end().map(|e| e.1[0].1), Some(false));
        s.begin((7, 10, 12), 1, 0x100);
        s.note(1, 0x1100, 32, 4, false);
        s.read(1, 0x1100 + 3 * 32);
        assert_eq!(s.end().map(|e| e.1[0].1), Some(true));
    }

    #[test]
    fn a_window_is_planned_once_read_twice_running_and_dropped_once_unread() {
        let mut k = Kept::default();
        k.round(&[(A, true, false)]);
        assert!(k.plan().is_empty(), "read once is not enough");
        k.round(&[(A, true, false), (B, true, false)]);
        assert_eq!(k.plan(), [A]);
        // planned and not read: out of the plan at once, still remembered
        k.round(&[(A, false, true), (B, true, false)]);
        assert_eq!(k.plan(), [B]);
        assert_eq!(k.windows.len(), 2);
        // four times round with no draw reading it, it is forgotten
        for _ in 0..4 {
            k.round(&[]);
        }
        assert!(k.windows.is_empty());
    }

    #[test]
    fn a_head_whose_plans_go_mostly_unread_rests() {
        let mut k = Kept::default();
        let many: Vec<Planned> = (1..=8).map(|i| Planned { delta: i * 0x100, step: 32, records: 4 }).collect();
        let read: Vec<_> = many.iter().map(|&p| (p, true, false)).collect();
        k.round(&read);
        k.round(&read);
        assert_eq!(k.plan().len(), 8);
        // eight planned each time round, two of them read
        for r in 0..8 {
            let seen: Vec<_> = many.iter().enumerate().map(|(i, &p)| (p, (i + r) % 4 == 0, true)).collect();
            k.round(&seen);
            if k.rest > 0 {
                break;
            }
        }
        assert!(k.rest > 0, "a head with fewer than half its planned windows read rests");
        assert!(k.plan().is_empty());
        let before = k.rest;
        k.round(&read);
        assert_eq!(k.rest, before - 1);
    }

    #[test]
    fn plans_are_kept_and_forgotten() {
        let key = (0xdead, 10, 12, 1);
        let read = [(A, true, false)];
        super::keep(key, &read);
        assert_eq!(super::plan(key), None);
        super::keep(key, &read);
        assert_eq!(super::plan(key), Some(vec![A]));
        for _ in 0..5 {
            super::keep(key, &[]);
        }
        assert_eq!(super::plan(key), None);
    }
}
