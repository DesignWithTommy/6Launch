//! VICTORY_TABLE_REUSE=1: per list, a table slot with unchanged record words reuses its view and buffer copy

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

pub(crate) fn on() -> bool {
    // live: the epoch moves with the switch off too, so notes stay true
    static S: crate::switch::Switch = crate::switch::Switch::new("TABLE_REUSE");
    S.on()
}

// moves whenever what a record names can change without its words changing
static EPOCH: AtomicU64 = AtomicU64::new(1);
static COUNTS: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];

pub(crate) fn bump() {
    EPOCH.fetch_add(1, Ordering::AcqRel);
}

pub(crate) fn now() -> u64 {
    EPOCH.load(Ordering::Acquire)
}

/// The three words a descriptor record names its view by.
pub(crate) fn record_at(handle: u64) -> [u32; 3] {
    // SAFETY: the handle is a descriptor slot in title memory, as `descriptor::view` reads it.
    unsafe { (handle as *const [u32; 3]).read_unaligned() }
}

/// The host slot key a record makes (`HostHeap::mirror`).
pub(crate) fn key(record: [u32; 3]) -> u64 {
    u64::from(record[2]) << 32 | u64::from(record[1])
}

/// A buffer view's host copy and range, as `descriptor::host::buffer_copy` gives it.
pub(crate) type HostCopy = (std::sync::Arc<crate::shadow::Shadow>, u64, u64);

struct Seen<V, C> {
    record: [u32; 3],
    epoch: u64,
    view: V,
    copy: Option<C>,
}

/// What this list's draws took from table slots, by title handle.
pub(crate) struct Reuse<V, C> {
    seen: HashMap<u64, Seen<V, C>>,
}

impl<V, C> Default for Reuse<V, C> {
    fn default() -> Self {
        Reuse { seen: HashMap::new() }
    }
}

// a list's worth of distinct slots; past it start again
const LIMIT: usize = 1 << 16;

impl<V: Clone, C: Clone> Reuse<V, C> {
    pub(crate) fn clear(&mut self) {
        self.seen.clear();
    }

    /// The view and copy taken from `handle` while it held `record`, if
    /// the epoch is still `now` and the copy is still `live`.
    pub(crate) fn get(&self, handle: u64, record: [u32; 3], now: u64, live: impl Fn(&C) -> bool) -> Option<(V, Option<C>)> {
        let hit = self
            .seen
            .get(&handle)
            .filter(|s| s.record == record && s.epoch == now && s.copy.as_ref().is_none_or(&live))
            .map(|s| (s.view.clone(), s.copy.clone()));
        count(hit.is_some());
        hit
    }

    /// Notes what `handle` gave; `epoch` is [`now`] from before it was asked.
    pub(crate) fn put(&mut self, handle: u64, record: [u32; 3], epoch: u64, view: V, copy: Option<C>) {
        if self.seen.len() >= LIMIT {
            self.seen.clear();
        }
        self.seen.insert(handle, Seen { record, epoch, view, copy });
    }
}

fn count(hit: bool) {
    let n = COUNTS[usize::from(hit)].fetch_add(1, Ordering::Relaxed) + 1;
    if hit && n.is_power_of_two() && n >= 1 << 16 {
        tracing::info!(
            hits = n,
            asks = n + COUNTS[0].load(Ordering::Relaxed),
            "table reuse: slots a list took before, given back as they were (VICTORY_TABLE_REUSE=1)"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;


    // a title table of slots, each a 32-byte record; builds counted per slot
    struct Table {
        slots: Vec<[u32; 8]>,
        built: Vec<u32>,
        // this test's own epoch (the shared one moves under other tests)
        epoch: u64,
    }

    impl Table {
        fn new(n: usize) -> Table {
            let slots = (0..n as u32).map(|i| [0x4549_5676, i, 1, 0, 0, 0, 0, 0]).collect();
            Table { slots, built: vec![0; n], epoch: 1 }
        }

        fn handle(&self, k: usize) -> u64 {
            self.slots[k].as_ptr() as u64
        }

        // what a draw does with its table: each slot taken through the cache
        fn draw(&mut self, reuse: &mut Reuse<u64, u32>, live: bool) -> Vec<u64> {
            (0..self.slots.len())
                .map(|k| {
                    let handle = self.handle(k);
                    let record = record_at(handle);
                    if let Some((view, _)) = reuse.get(handle, record, self.epoch, |_| live) {
                        return view;
                    }
                    let epoch = self.epoch;
                    self.built[k] += 1;
                    let view = key(record);
                    reuse.put(handle, record, epoch, view, Some(7));
                    view
                })
                .collect()
        }
    }

    #[test]
    fn an_unchanged_table_is_built_once_per_list() {
        let mut t = Table::new(4);
        let mut reuse = Reuse::default();
        let first = t.draw(&mut reuse, true);
        assert_eq!(t.draw(&mut reuse, true), first);
        assert_eq!(t.built, [1, 1, 1, 1]);
    }

    #[test]
    fn a_descriptor_rewritten_behind_an_unchanged_table_is_rebuilt() {
        let mut t = Table::new(4);
        let mut reuse = Reuse::default();
        t.draw(&mut reuse, true);
        // the table handle stays, one of its records names another view
        t.slots[2][1] = 99;
        t.slots[2][2] = 5;
        let views = t.draw(&mut reuse, true);
        assert_eq!(t.built, [1, 1, 2, 1]);
        assert_eq!(views[2], 5 << 32 | 99);
        // only the generation moves: rebuilt too
        t.slots[1][2] = 2;
        t.draw(&mut reuse, true);
        assert_eq!(t.built, [1, 2, 2, 1]);
    }

    #[test]
    fn a_reset_a_release_or_a_dead_copy_rebuilds() {
        let mut t = Table::new(3);
        let mut reuse = Reuse::default();
        t.draw(&mut reuse, true);
        reuse.clear();
        t.draw(&mut reuse, true);
        assert_eq!(t.built, [2, 2, 2]);
        t.epoch += 1;
        t.draw(&mut reuse, true);
        assert_eq!(t.built, [3, 3, 3]);
        t.draw(&mut reuse, false);
        assert_eq!(t.built, [4, 4, 4]);
    }

    #[test]
    fn a_bump_while_building_is_not_hidden() {
        let t = Table::new(1);
        let mut reuse: Reuse<u64, u32> = Reuse::default();
        let handle = t.handle(0);
        let record = record_at(handle);
        let before = now();
        bump();
        reuse.put(handle, record, before, 1, None);
        assert!(now() > before);
        assert!(reuse.get(handle, record, now(), |_| true).is_none());
    }
}
