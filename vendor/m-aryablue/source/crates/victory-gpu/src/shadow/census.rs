// diagnostics with VICTORY_INPLACE_PROBE>=1: host copies summed by class every 10 s; a meter only, picks nothing

use std::sync::atomic::{AtomicU8, AtomicU64, Ordering::Relaxed};

use super::Shadow;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum Kind {
    Indirect = 0,
    Written = 1,
    Heap = 2,
    Constant = 3,
    VertexIndex = 4,
    View = 5,
}

const CLASSES: [&str; 7] = [
    "indirect arguments",
    "written by shaders",
    "through the descriptor heap",
    "constants",
    "vertex or index data",
    "buffer views",
    "none of these (windows and the rest)",
];

/// A copy's counts since the last line.
#[derive(Default)]
pub(crate) struct Meter {
    kinds: AtomicU8,
    checked: AtomicU64,
    check_ns: AtomicU64,
    copied: AtomicU64,
    asked: AtomicU64,
    back: AtomicU64,
    // the last frame (plus one) the card wrote it, and such frames
    gpu_at: AtomicU64,
    gpu_frames: AtomicU64,
}

pub(crate) fn on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    cfg!(feature = "diagnostics") && *ON.get_or_init(|| crate::inplace::probe_level() >= 1)
}

pub(crate) fn kind(shadow: &Shadow, kind: Kind) {
    if on() {
        shadow.census.kinds.fetch_or(1 << kind as u8, Relaxed);
    }
}

pub(crate) fn asked(shadow: &Shadow, bytes: u64) {
    if on() {
        shadow.census.asked.fetch_add(bytes, Relaxed);
    }
}

pub(crate) fn back(shadow: &Shadow, bytes: usize) {
    if on() {
        shadow.census.back.fetch_add(bytes as u64, Relaxed);
    }
}

pub(crate) fn card_wrote(shadow: &Shadow) {
    if on() {
        let m = &shadow.census;
        m.kinds.fetch_or(1 << Kind::Written as u8, Relaxed);
        let frame = crate::frame::current() + 1;
        if m.gpu_at.swap(frame, Relaxed) != frame {
            m.gpu_frames.fetch_add(1, Relaxed);
        }
    }
}

/// A refresh's check of `len` bytes, timed; `copied` bytes of it came in.
pub(crate) fn checked<R>(shadow: &Shadow, len: usize, check: impl FnOnce() -> R, copied: impl Fn(&R) -> usize) -> R {
    if !on() {
        return check();
    }
    let t = std::time::Instant::now();
    let r = check();
    let m = &shadow.census;
    m.check_ns.fetch_add(t.elapsed().as_nanos() as u64, Relaxed);
    m.checked.fetch_add(len as u64, Relaxed);
    m.copied.fetch_add(copied(&r) as u64, Relaxed);
    r
}

/// One copy's counts, taken (set to zero) for a line.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub(crate) struct Sample {
    pub(crate) kinds: u8,
    pub(crate) size: u64,
    pub(crate) checked: u64,
    pub(crate) check_ns: u64,
    pub(crate) copied: u64,
    pub(crate) asked: u64,
    pub(crate) back: u64,
    pub(crate) gpu_frames: u64,
}

impl Meter {
    fn take(&self, size: u64) -> Sample {
        Sample {
            // kinds stay: a binding seen once is the copy's class
            kinds: self.kinds.load(Relaxed),
            size,
            checked: self.checked.swap(0, Relaxed),
            check_ns: self.check_ns.swap(0, Relaxed),
            copied: self.copied.swap(0, Relaxed),
            asked: self.asked.swap(0, Relaxed),
            back: self.back.swap(0, Relaxed),
            gpu_frames: self.gpu_frames.swap(0, Relaxed),
        }
    }
}

/// The class of a copy with these kinds: the lowest kind set, else none.
pub(crate) fn class(kinds: u8) -> usize {
    if kinds == 0 { CLASSES.len() - 1 } else { (kinds.trailing_zeros() as usize).min(CLASSES.len() - 1) }
}

/// Per class: copies and the sums of their samples.
pub(crate) fn sum(samples: &[Sample]) -> [(u64, Sample); 7] {
    let mut rows = [(0u64, Sample::default()); 7];
    for s in samples {
        let (n, r) = &mut rows[class(s.kinds)];
        *n += 1;
        r.kinds |= s.kinds;
        r.size += s.size;
        r.checked += s.checked;
        r.check_ns += s.check_ns;
        r.copied += s.copied;
        r.asked += s.asked;
        r.back += s.back;
        r.gpu_frames += s.gpu_frames;
    }
    rows
}

/// Once a second (`crate::inplace::tick`): a line per class every 10 s.
pub(crate) fn tick(copies: impl FnOnce() -> Vec<std::sync::Arc<Shadow>>) {
    static LAST: std::sync::Mutex<Option<(u64, u64)>> = std::sync::Mutex::new(None);
    if !on() {
        return;
    }
    let now = (crate::frame::current(), crate::host::now_ms());
    let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
    let Some((frame, ms)) = *last else {
        *last = Some(now);
        // the counts so far are from before the first line
        copies().iter().for_each(|s| {
            s.census.take(0);
        });
        return;
    };
    if now.1 < ms + 10_000 {
        return;
    }
    *last = Some(now);
    let frames = now.0.saturating_sub(frame).max(1) as f64;
    let samples: Vec<Sample> = copies().iter().map(|s| s.census.take(s.len)).collect();
    let mb = |b: u64| b as f64 / (1 << 20) as f64;
    for (i, (n, r)) in sum(&samples).iter().enumerate() {
        if *n == 0 {
            continue;
        }
        tracing::info!(
            "copies by class (VICTORY_INPLACE_PROBE census), {}: {n} copies {:.0} MB; per frame: checked {:.1} MB in {:.2} ms, copied in {:.2} MB, \
             given to shaders {:.1} MB, copied back {:.2} MB, {:.1} copies written by the card",
            CLASSES[i],
            mb(r.size),
            mb(r.checked) / frames,
            r.check_ns as f64 / 1e6 / frames,
            mb(r.copied) / frames,
            mb(r.asked) / frames,
            mb(r.back) / frames,
            r.gpu_frames as f64 / frames,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copies_sum_into_their_first_class() {
        let k = |kinds: &[Kind]| kinds.iter().fold(0u8, |m, &k| m | 1 << k as u8);
        assert_eq!(class(0), 6);
        assert_eq!(class(k(&[Kind::View])), 5);
        assert_eq!(class(k(&[Kind::View, Kind::Constant])), 3);
        assert_eq!(class(k(&[Kind::Heap, Kind::VertexIndex, Kind::Written])), 1);
        assert_eq!(class(k(&[Kind::Indirect, Kind::Heap])), 0);
        let s = |kinds: u8, size: u64, checked: u64| Sample { kinds, size, checked, check_ns: 10, copied: 1, asked: 2, back: 3, gpu_frames: 4 };
        let rows = sum(&[
            s(k(&[Kind::Heap]), 100, 1000),
            s(k(&[Kind::Heap, Kind::View]), 50, 500),
            s(k(&[Kind::Constant]), 7, 70),
            s(0, 9, 0),
        ]);
        assert_eq!(rows[2].0, 2);
        assert_eq!((rows[2].1.size, rows[2].1.checked, rows[2].1.check_ns, rows[2].1.back, rows[2].1.gpu_frames), (150, 1500, 20, 6, 8));
        assert_eq!(rows[3].0, 1);
        assert_eq!(rows[3].1.size, 7);
        assert_eq!(rows[6].0, 1);
        assert_eq!(rows[6].1.size, 9);
        assert_eq!(rows.iter().map(|r| r.0).sum::<u64>(), 4);
        assert_eq!(rows.iter().map(|r| r.1.size).sum::<u64>(), 166);
    }

    #[test]
    fn a_meter_is_taken_once() {
        let m = Meter::default();
        m.checked.fetch_add(5, Relaxed);
        m.kinds.fetch_or(1 << Kind::View as u8, Relaxed);
        let a = m.take(10);
        assert_eq!((a.checked, a.size, a.kinds), (5, 10, 1 << 5));
        let b = m.take(10);
        assert_eq!((b.checked, b.kinds), (0, 1 << 5), "counts restart; kinds stay");
    }
}
