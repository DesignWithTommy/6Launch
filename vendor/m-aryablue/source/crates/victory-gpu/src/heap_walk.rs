// the heap-buffer pass's slot walk on a worker thread (VICTORY_HEAP_WALK_THREAD=1)
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

/// A host copy and the range of it a slot (or a span of slots) reads.
pub(crate) type Range<T> = (Arc<T>, u64, u64);

pub(crate) fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("VICTORY_HEAP_WALK_THREAD").is_ok_and(|v| v == "1"))
}

/// What the walk reads: `None` once the heap is gone.
pub(crate) trait Source<T>: Send + 'static {
    fn changes(&mut self) -> Option<Vec<u32>>;
    fn slots(&mut self) -> Option<Vec<u32>>;
    /// Walks `slots` into `ranges` (each slot dropped, then put back if it
    /// still reads a copy); false once the heap is gone.
    fn walk(&mut self, slots: &[u32], ranges: &mut HashMap<u32, Range<T>>) -> bool;
    // slots whose copy went stale since the last round (walked again)
    fn stale(&mut self, _ranges: &HashMap<u32, Range<T>>) -> Vec<u32> {
        Vec::new()
    }
}

/// What the worker hands the recording thread: per copy, the span its
/// slots read, as of the last walk.
pub(crate) struct Shared<T> {
    spans: Mutex<Option<Arc<Vec<Range<T>>>>>,
    ready: Condvar,
    passes: AtomicU64,
    gone: AtomicBool,
    pub(crate) walked: AtomicU64,
    pub(crate) walk_us: AtomicU64,
}

impl<T> Shared<T> {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            spans: Mutex::new(None),
            ready: Condvar::new(),
            passes: AtomicU64::new(0),
            gone: AtomicBool::new(false),
            walked: AtomicU64::new(0),
            walk_us: AtomicU64::new(0),
        })
    }

    /// The recording thread's pass: counts it (a full walk every 16) and
    /// gives the latest spans, waiting up to `wait` for the first.
    pub(crate) fn pass(&self, wait: Duration) -> Option<Arc<Vec<Range<T>>>> {
        self.passes.fetch_add(1, Ordering::Relaxed);
        let spans = self.spans.lock().unwrap_or_else(|e| e.into_inner());
        let (spans, _) = self
            .ready
            .wait_timeout_while(spans, wait, |s| s.is_none() && !self.gone.load(Ordering::Relaxed))
            .unwrap_or_else(|e| e.into_inner());
        spans.clone()
    }

    pub(crate) fn gone(&self) -> bool {
        self.gone.load(Ordering::Relaxed)
    }

    fn publish(&self, spans: Vec<Range<T>>) {
        *self.spans.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(spans));
        self.ready.notify_all();
    }
}

/// Per copy, the span its slots' ranges cover.
pub(crate) fn spans<T>(ranges: &HashMap<u32, Range<T>>) -> Vec<Range<T>> {
    let mut spans: HashMap<usize, Range<T>> = HashMap::new();
    for (copy, from, to) in ranges.values() {
        let span = spans
            .entry(Arc::as_ptr(copy) as *const () as usize)
            .or_insert_with(|| (copy.clone(), *from, *to));
        span.1 = span.1.min(*from);
        span.2 = span.2.max(*to);
    }
    spans.into_values().collect()
}

/// The worker: drains the changed slots in chunks, all slots every 16
/// passes (a view of a released resource goes stale without a write), and
/// publishes the spans after each round that changed anything.
pub(crate) fn run<T, S: Source<T>>(mut source: S, shared: Arc<Shared<T>>, chunk: usize, idle: Duration) {
    let mut ranges: HashMap<u32, Range<T>> = HashMap::new();
    let mut full_at: Option<u64> = None;
    'walk: loop {
        let started = std::time::Instant::now();
        let passes = shared.passes.load(Ordering::Relaxed);
        let mut changed = false;
        if full_at.is_none_or(|at| passes / 16 != at / 16) {
            let Some(slots) = source.slots() else { break 'walk };
            for part in slots.chunks(chunk.max(1)) {
                if !source.walk(part, &mut ranges) {
                    break 'walk;
                }
            }
            let seen: HashSet<u32> = slots.iter().copied().collect();
            ranges.retain(|slot, _| seen.contains(slot));
            shared.walked.fetch_add(slots.len() as u64, Ordering::Relaxed);
            full_at = Some(passes);
            changed = true;
        }
        let Some(mut slots) = source.changes() else { break 'walk };
        slots.extend(source.stale(&ranges));
        for part in slots.chunks(chunk.max(1)) {
            if !source.walk(part, &mut ranges) {
                break 'walk;
            }
        }
        shared.walked.fetch_add(slots.len() as u64, Ordering::Relaxed);
        changed |= !slots.is_empty();
        if changed {
            shared.publish(spans(&ranges));
            shared.walk_us.fetch_add(started.elapsed().as_micros() as u64, Ordering::Relaxed);
        }
        // a pause between rounds lets changes gather
        std::thread::sleep(idle);
    }
    shared.gone.store(true, Ordering::Relaxed);
    *shared.spans.lock().unwrap_or_else(|e| e.into_inner()) = None;
    shared.ready.notify_all();
}

#[cfg(test)]
mod tests {
    use super::*;

    // a fake heap: slot -> (copy, from, to), with changes queued by the test
    struct Fake {
        slots: Arc<Mutex<(HashMap<u32, (usize, u64, u64)>, Vec<u32>, bool)>>,
        copies: Vec<Arc<usize>>,
    }

    impl Source<usize> for Fake {
        fn changes(&mut self) -> Option<Vec<u32>> {
            let mut s = self.slots.lock().unwrap();
            (!s.2).then(|| std::mem::take(&mut s.1))
        }
        fn slots(&mut self) -> Option<Vec<u32>> {
            let s = self.slots.lock().unwrap();
            (!s.2).then(|| s.0.keys().copied().collect())
        }
        fn walk(&mut self, slots: &[u32], ranges: &mut HashMap<u32, Range<usize>>) -> bool {
            let s = self.slots.lock().unwrap();
            for slot in slots {
                ranges.remove(slot);
                if let Some(&(copy, from, to)) = s.0.get(slot) {
                    ranges.insert(*slot, (self.copies[copy].clone(), from, to));
                }
            }
            !s.2
        }
    }

    fn sorted(spans: &[Range<usize>]) -> Vec<(usize, u64, u64)> {
        let mut v: Vec<_> = spans.iter().map(|(c, a, b)| (**c, *a, *b)).collect();
        v.sort();
        v
    }

    fn until(shared: &Shared<usize>, want: &[(usize, u64, u64)]) -> bool {
        for _ in 0..500 {
            if let Some(s) = shared.pass(Duration::from_secs(1))
                && sorted(&s) == want
            {
                return true;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        false
    }

    // a fake whose copy 0 goes stale once: slots holding it are walked again without a change
    struct Stale(Fake, Arc<AtomicBool>);

    impl Source<usize> for Stale {
        fn changes(&mut self) -> Option<Vec<u32>> {
            self.0.changes()
        }
        fn slots(&mut self) -> Option<Vec<u32>> {
            self.0.slots()
        }
        fn walk(&mut self, slots: &[u32], ranges: &mut HashMap<u32, Range<usize>>) -> bool {
            self.0.walk(slots, ranges)
        }
        fn stale(&mut self, ranges: &HashMap<u32, Range<usize>>) -> Vec<u32> {
            if !self.1.swap(false, Ordering::Relaxed) {
                return Vec::new();
            }
            ranges.iter().filter(|(_, r)| *r.0 == 0).map(|(&i, _)| i).collect()
        }
    }

    #[test]
    fn stale_slots_are_walked_again_without_a_change() {
        let heap = Arc::new(Mutex::new((HashMap::new(), Vec::new(), false)));
        heap.lock().unwrap().0.insert(1, (0, 0x100, 0x200));
        let shared = Shared::new();
        let stale = Arc::new(AtomicBool::new(false));
        let fake = Stale(Fake { slots: heap.clone(), copies: vec![Arc::new(0), Arc::new(1)] }, stale.clone());
        let worker = {
            let shared = shared.clone();
            std::thread::spawn(move || run(fake, shared, 2, Duration::from_millis(1)))
        };
        assert_eq!(sorted(&shared.pass(Duration::from_secs(5)).expect("first walk")), vec![(0, 0x100, 0x200)]);
        // the slot now reads copy 1 (as a swapped copy would), with no change queued
        heap.lock().unwrap().0.insert(1, (1, 0x100, 0x200));
        stale.store(true, Ordering::Relaxed);
        // under 16 passes, so no full walk can be what finds it
        let mut found = false;
        for _ in 0..10 {
            std::thread::sleep(Duration::from_millis(30));
            if shared.pass(Duration::ZERO).is_some_and(|s| sorted(&s) == [(1, 0x100, 0x200)]) {
                found = true;
                break;
            }
        }
        assert!(found, "the stale slot was not walked again");
        heap.lock().unwrap().2 = true;
        worker.join().unwrap();
    }

    #[test]
    fn the_worker_walks_changes_off_the_recording_thread_and_hands_over_spans() {
        let heap = Arc::new(Mutex::new((HashMap::new(), Vec::new(), false)));
        {
            let mut h = heap.lock().unwrap();
            h.0.insert(1, (0, 0x100, 0x200));
            h.0.insert(2, (0, 0x400, 0x500));
            h.0.insert(3, (1, 0x10, 0x20));
        }
        let shared = Shared::new();
        let fake = Fake { slots: heap.clone(), copies: vec![Arc::new(0), Arc::new(1)] };
        let worker = {
            let shared = shared.clone();
            std::thread::spawn(move || run(fake, shared, 2, Duration::from_millis(1)))
        };
        // the first pass waits for the first full walk
        let first = shared.pass(Duration::from_secs(5)).expect("spans after the first walk");
        assert_eq!(sorted(&first), vec![(0, 0x100, 0x500), (1, 0x10, 0x20)]);
        // a changed slot is walked by the worker alone
        {
            let mut h = heap.lock().unwrap();
            h.0.insert(3, (1, 0x10, 0x80));
            h.1.push(3);
        }
        assert!(until(&shared, &[(0, 0x100, 0x500), (1, 0x10, 0x80)]));
        // a slot gone without a change is dropped by the full walk
        heap.lock().unwrap().0.remove(&2);
        for _ in 0..16 {
            shared.pass(Duration::ZERO);
        }
        assert!(until(&shared, &[(0, 0x100, 0x200), (1, 0x10, 0x80)]));
        // the heap gone: the worker ends and hands nothing over
        heap.lock().unwrap().2 = true;
        worker.join().unwrap();
        assert!(shared.gone());
        assert!(shared.pass(Duration::ZERO).is_none());
    }
}
