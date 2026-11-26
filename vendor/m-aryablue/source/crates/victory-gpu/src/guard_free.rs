//! `VICTORY_ONWRITE_LOCKFREE=1` (off by default; on in the tests): the
//! guards' fault handler opens a block without the guards' lock. Beside the
//! guards' `protected` set a bitmap marks the blocks that are surely ours
//! and read-only; the handler clears the block's bit (taking it), makes it
//! read-write, and puts it on a bounded queue that the next holder of the
//! guards' lock drains before anything else, opening it there as the
//! locked handler did. No allocation and no lock in the handler: a block
//! not marked (or a full queue) goes the locked way as before.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicPtr, AtomicU64, AtomicUsize, Ordering};

// blocks to a leaf of the bitmap: 2 GiB of 64 KiB blocks
const LEAF_BITS: usize = 15;
const LEAF_WORDS: usize = (1 << LEAF_BITS) / 64;
// user addresses lie below 2^47; a block is 2^16 bytes
const TOP: usize = 1 << (47 - 16 - LEAF_BITS);
const RING: usize = 1 << 14;

type Leaf = [AtomicU64; LEAF_WORDS];

struct Slot {
    seq: AtomicUsize,
    block: AtomicUsize,
    frame: AtomicU64,
}

struct Ring {
    slots: Box<[Slot]>,
    head: AtomicUsize,
    tail: AtomicUsize,
}

struct Tables {
    top: Box<[AtomicPtr<Leaf>]>,
    ring: Ring,
}

static TABLES: OnceLock<Option<Tables>> = OnceLock::new();

/// Since the start: blocks the handler opened without the lock, and those
/// that then found the queue full.
pub(crate) static TAKEN: [AtomicU64; 2] = [const { AtomicU64::new(0) }; 2];

/// Before the handler is installed: reads the switch and makes the tables.
pub(crate) fn init() {
    TABLES.get_or_init(|| {
        let on = cfg!(test) || std::env::var_os("VICTORY_ONWRITE_LOCKFREE").is_some_and(|v| v == "1");
        if !on {
            return None;
        }
        tracing::info!("the guards' fault handler opens blocks without the guards' lock (VICTORY_ONWRITE_LOCKFREE=1)");
        Some(Tables {
            top: (0..TOP).map(|_| AtomicPtr::new(std::ptr::null_mut())).collect(),
            ring: Ring::new(),
        })
    });
}

fn tables() -> Option<&'static Tables> {
    TABLES.get().and_then(Option::as_ref)
}

pub(crate) fn on() -> bool {
    tables().is_some()
}

fn word(t: &Tables, block: usize) -> Option<(&AtomicU64, u64)> {
    let leaf = t.top.get(block >> LEAF_BITS)?.load(Ordering::Acquire);
    if leaf.is_null() {
        return None;
    }
    // SAFETY: leaves are made once and never freed.
    let leaf = unsafe { &*leaf };
    Some((&leaf[(block >> 6) & (LEAF_WORDS - 1)], 1u64 << (block & 63)))
}

/// Under the guards' lock, once `block` is read-only and in `protected`.
pub(crate) fn mark(block: usize) {
    let Some(t) = tables() else {
        return;
    };
    let Some(slot) = t.top.get(block >> LEAF_BITS) else {
        return;
    };
    if slot.load(Ordering::Acquire).is_null() {
        let leaf: *mut Leaf = Box::into_raw(Box::new([const { AtomicU64::new(0) }; LEAF_WORDS]));
        if slot.compare_exchange(std::ptr::null_mut(), leaf, Ordering::AcqRel, Ordering::Acquire).is_err() {
            // SAFETY: ours, never shared.
            drop(unsafe { Box::from_raw(leaf) });
        }
    }
    if let Some((w, bit)) = word(t, block) {
        w.fetch_or(bit, Ordering::Release);
    }
}

pub(crate) fn mark_range(from: usize, to: usize) {
    if on() {
        (from..to).for_each(mark);
    }
}

/// Under the guards' lock, as `block` leaves `protected`.
pub(crate) fn unmark(block: usize) {
    if let Some((w, bit)) = tables().and_then(|t| word(t, block)) {
        w.fetch_and(!bit, Ordering::AcqRel);
    }
}

// handler calls between before their take and after their push
static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);

/// A handler call in flight ([`IN_FLIGHT`]) while held; none when off.
pub(crate) struct Flight(());

impl Flight {
    pub(crate) fn enter() -> Option<Flight> {
        on().then(|| {
            IN_FLIGHT.fetch_add(1, Ordering::SeqCst);
            Flight(())
        })
    }
}

impl Drop for Flight {
    fn drop(&mut self) {
        IN_FLIGHT.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Under the guards' lock, before the drain: a block taken (read-write
/// now) but not pushed yet would be read as clean by the pass.
pub(crate) fn wait_flights() {
    let mut spins = 0u32;
    while IN_FLIGHT.load(Ordering::SeqCst) != 0 {
        spins += 1;
        if spins < 64 {
            std::hint::spin_loop();
        } else {
            std::thread::yield_now();
        }
    }
}

#[cfg(test)]
pub(crate) static PARKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
#[cfg(test)]
pub(crate) static GO: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

// a test's writer stops between its take and its push until GO
#[cfg(test)]
pub(crate) fn park_here() {
    if PARK.with(|p| p.get()) {
        PARKED.store(true, Ordering::SeqCst);
        while !GO.load(Ordering::SeqCst) {
            std::thread::yield_now();
        }
    }
}

#[cfg(test)]
thread_local! {
    pub(crate) static PARK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(test)]
thread_local! {
    // a test's writer threads that fault the locked way
    pub(crate) static LOCKED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// In the handler: whether this thread takes `block` (its bit was set and
/// is now clear). Then it is the caller's to open and [`push`].
pub(crate) fn take(block: usize) -> bool {
    #[cfg(test)]
    if LOCKED.with(|l| l.get()) {
        return false;
    }
    tables()
        .and_then(|t| word(t, block))
        .is_some_and(|(w, bit)| w.fetch_and(!bit, Ordering::AcqRel) & bit != 0)
}

impl Ring {
    fn new() -> Ring {
        Ring {
            slots: (0..RING)
                .map(|i| Slot { seq: AtomicUsize::new(i), block: AtomicUsize::new(0), frame: AtomicU64::new(0) })
                .collect(),
            head: AtomicUsize::new(0),
            tail: AtomicUsize::new(0),
        }
    }

    fn push(&self, block: usize, frame: u64) -> bool {
        let mut pos = self.tail.load(Ordering::Relaxed);
        loop {
            let slot = &self.slots[pos & (RING - 1)];
            let seq = slot.seq.load(Ordering::Acquire);
            let dif = seq as isize - pos as isize;
            if dif == 0 {
                match self.tail.compare_exchange_weak(pos, pos + 1, Ordering::Relaxed, Ordering::Relaxed) {
                    Ok(_) => {
                        slot.block.store(block, Ordering::Relaxed);
                        slot.frame.store(frame, Ordering::Relaxed);
                        slot.seq.store(pos + 1, Ordering::Release);
                        return true;
                    }
                    Err(now) => pos = now,
                }
            } else if dif < 0 {
                return false;
            } else {
                pos = self.tail.load(Ordering::Relaxed);
            }
        }
    }

    // one drainer at a time (the guards' lock): no race for the head
    fn drain(&self, mut f: impl FnMut(usize, u64)) {
        let mut pos = self.head.load(Ordering::Relaxed);
        loop {
            let slot = &self.slots[pos & (RING - 1)];
            if slot.seq.load(Ordering::Acquire) != pos + 1 {
                break;
            }
            let (block, frame) = (slot.block.load(Ordering::Relaxed), slot.frame.load(Ordering::Relaxed));
            slot.seq.store(pos + RING, Ordering::Release);
            pos += 1;
            self.head.store(pos, Ordering::Relaxed);
            f(block, frame);
        }
    }
}

/// In the handler: a taken block, made read-write, for the next drain.
/// False if the queue is full (the caller opens it under the lock).
pub(crate) fn push(block: usize, frame: u64) -> bool {
    let done = tables().is_some_and(|t| t.ring.push(block, frame));
    TAKEN[usize::from(!done)].fetch_add(1, Ordering::Relaxed);
    done
}

/// Under the guards' lock: every block pushed so far, in order, with the
/// frame it was written in. One still being pushed is the next drain's.
pub(crate) fn drain(f: impl FnMut(usize, u64)) {
    if let Some(t) = tables() {
        t.ring.drain(f);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // writers on many threads; one drainer; nothing lost, nothing twice, a full queue says so
    #[test]
    fn pushed_blocks_are_all_drained_once() {
        let ring = std::sync::Arc::new(Ring::new());
        let base = 0x7fe0_0000_0000usize / (64 << 10);
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let drainer = {
            let (seen, stop, ring) = (seen.clone(), stop.clone(), ring.clone());
            std::thread::spawn(move || {
                loop {
                    let done = stop.load(Ordering::SeqCst);
                    {
                        let mut seen = seen.lock().unwrap();
                        ring.drain(|b, _| seen.push(b));
                    }
                    if done {
                        break;
                    }
                    std::thread::yield_now();
                }
            })
        };
        let writers: Vec<_> = (0..8usize)
            .map(|t| {
                let ring = ring.clone();
                std::thread::spawn(move || {
                    let mut lost = Vec::new();
                    for i in 0..20_000usize {
                        let block = base + t * 1_000_000 + i;
                        if !ring.push(block, 0) {
                            lost.push(block);
                        }
                    }
                    lost
                })
            })
            .collect();
        let lost: Vec<usize> = writers.into_iter().flat_map(|w| w.join().unwrap()).collect();
        stop.store(true, Ordering::SeqCst);
        drainer.join().unwrap();
        let mut seen: Vec<usize> = seen.lock().unwrap().clone();
        seen.sort_unstable();
        let mut all = seen.clone();
        all.extend(&lost);
        all.sort_unstable();
        all.dedup();
        assert_eq!(all.len(), seen.len() + lost.len(), "a block drained twice or both drained and refused");
        assert_eq!(all.len(), 8 * 20_000, "a block lost");
    }

    #[test]
    fn a_block_is_taken_once_and_only_while_marked() {
        init();
        let block = 0x7fd0_0000_0000usize / (64 << 10) + 5;
        assert!(!take(block), "never marked");
        mark(block);
        mark(block + 1);
        assert!(take(block));
        assert!(!take(block), "taken already");
        unmark(block + 1);
        assert!(!take(block + 1));
        mark(block + 3);
        let won: usize = (0..8).map(|_| std::thread::spawn(move || take(block + 3))).map(|t| usize::from(t.join().unwrap())).sum();
        assert_eq!(won, 1, "one thread takes a marked block");
        assert!(!take(usize::MAX), "past the address space");
    }
}
