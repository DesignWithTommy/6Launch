//! What the map tour (`victory_hle::tour`) reads of the frames and asks of
//! them: frame and draw counters, textures and pipelines still waiting, and
//! one frame saved on request (into `VICTORY_CAPTURE`).

use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

/// Frames presented so far.
pub fn presented() -> u64 {
    crate::queue::presented()
}

/// Draws run on the host so far.
pub fn drawn() -> u64 {
    crate::draw::drawn()
}

/// Textures picked by index that still wait for their upload.
pub fn textures_waiting() -> u64 {
    crate::draw::HEAP_TEXTURES_WAITING.load(Relaxed)
}

/// Host pipelines queued and not made yet.
pub fn pipelines_waiting() -> u64 {
    crate::pipeline::making()
}

/// The frame the tour wants saved.
static SNAP: AtomicU64 = AtomicU64::new(u64::MAX);

/// Saves frame `frame` as `frame-<frame>.png` (with `VICTORY_CAPTURE`).
pub fn save_frame(frame: u64) {
    SNAP.store(frame, Relaxed);
}

/// Whether the tour asked for frame `n`.
pub(crate) fn wanted(n: u64) -> bool {
    SNAP.load(Relaxed) == n
}
