//! VICTORY_READABLE_LEAN=1: what a list already has for a draw's read is not asked for again

use std::cell::Cell;
use std::sync::atomic::{AtomicU64, Ordering};

pub(crate) fn on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_READABLE_LEAN").is_some_and(|v| v == "1"))
}

// never the same twice, so a list's old marks never match its new ones
static NEXT: AtomicU64 = AtomicU64::new(1);
pub(crate) static COUNTS: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];

/// A fresh mark for a list's refresh set or state table, taken whenever either loses what it held.
pub(crate) fn fresh() -> u64 {
    NEXT.fetch_add(1, Ordering::Relaxed)
}

const SLOTS: usize = 512;

// direct mapped: (mark, key, state); a wrong miss only costs the slow path
struct Seen {
    slots: [Cell<(u64, [usize; 3], u32)>; SLOTS],
}

thread_local! {
    static REFRESHED: Seen = const { Seen { slots: [const { Cell::new((0, [0; 3], 0)) }; SLOTS] } };
    static STATES: Seen = const { Seen { slots: [const { Cell::new((0, [0; 3], 0)) }; SLOTS] } };
}

fn slot(key: [usize; 3]) -> usize {
    let h = (key[0] as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15)
        ^ (key[1] as u64).rotate_left(21).wrapping_mul(0xc2b2_ae3d_27d4_eb4f)
        ^ (key[2] as u64).wrapping_mul(0x1656_67b1_9e37_79f9);
    (h >> 40) as usize % SLOTS
}

impl Seen {
    fn has(&self, mark: u64, key: [usize; 3], state: u32) -> bool {
        mark != 0 && self.slots[slot(key)].get() == (mark, key, state)
    }

    fn note(&self, mark: u64, key: [usize; 3], state: u32) {
        self.slots[slot(key)].set((mark, key, state));
    }
}

/// Whether this list (by its refresh mark) already holds the refresh of `key`.
pub(crate) fn refreshed(mark: u64, key: (usize, usize, usize)) -> bool {
    let hit = REFRESHED.with(|s| s.has(mark, [key.0, key.1, key.2], 0));
    if hit {
        COUNTS[0].fetch_add(1, Ordering::Relaxed);
    }
    hit
}

pub(crate) fn note_refreshed(mark: u64, key: (usize, usize, usize)) {
    REFRESHED.with(|s| s.note(mark, [key.0, key.1, key.2], 0));
}

/// Whether this list (by its state mark) already has host copy `copy` in `state`.
pub(crate) fn in_state(mark: u64, copy: usize, state: u32) -> bool {
    let hit = STATES.with(|s| s.has(mark, [copy, 0, 0], state));
    let n = COUNTS[if hit { 1 } else { 2 }].fetch_add(1, Ordering::Relaxed);
    if hit && n.is_multiple_of(1_000_000) {
        tracing::info!(
            states_kept = n + 1,
            states_asked = COUNTS[2].load(Ordering::Relaxed),
            refreshes_kept = COUNTS[0].load(Ordering::Relaxed),
            "readable lean"
        );
    }
    hit
}

pub(crate) fn note_state(mark: u64, copy: usize, state: u32) {
    STATES.with(|s| s.note(mark, [copy, 0, 0], state));
}

/// `crate::list::ensure_state` for a host copy already looked up, skipped when the list has it so.
pub(crate) fn ensure(list: &crate::host::HostList, copy: &std::sync::Arc<crate::resource::HostTexture>, state: u32) {
    use windows::Win32::Graphics::Direct3D12::D3D12_RESOURCE_STATES as State;
    let wanted = crate::host::legal_state(&copy.texture, state);
    let key = std::sync::Arc::as_ptr(copy) as usize;
    if in_state(list.states_mark(), key, wanted) {
        return;
    }
    if let Some(before) = list.transition_copy(copy, wanted) {
        let barrier = crate::host::transition(&copy.texture, State(before as i32), State(wanted as i32));
        // SAFETY: an open list and a live texture.
        unsafe { list.list.ResourceBarrier(&[barrier]) };
    }
    note_state(list.states_mark(), key, wanted);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mark_taken_after_a_clear_forgets_what_was_seen() {
        let a = fresh();
        let key = (0x1000, 0x2000, 0x40);
        assert!(!refreshed(a, key));
        note_refreshed(a, key);
        assert!(refreshed(a, key));
        assert!(!refreshed(a, (0x1000, 0x2000, 0x80)));
        let b = fresh();
        assert!(!refreshed(b, key));
    }

    #[test]
    fn a_state_is_kept_only_for_its_mark_and_value() {
        let a = fresh();
        note_state(a, 0x5000, 0x40);
        assert!(in_state(a, 0x5000, 0x40));
        assert!(!in_state(a, 0x5000, 0x80));
        assert!(!in_state(fresh(), 0x5000, 0x40));
        assert!(!in_state(0, 0, 0));
    }
}
