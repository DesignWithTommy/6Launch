//! Copies of filled textures held while the title holds them (`VICTORY_TEXTURE_HOLD=1`), and
//! textures read again as DirectStorage lands their bytes (`VICTORY_TEXTURE_ARRIVE=1`).

#[cfg(feature = "diagnostics")]
use std::collections::HashMap;
#[cfg(feature = "diagnostics")]
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

/// Whether copies are held while the title holds them.
pub(crate) fn on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_TEXTURE_HOLD").is_some_and(|v| v == "1"))
}

/// Whether DirectStorage landing in a placed texture's memory makes it ready at once.
pub(crate) fn arrive() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_TEXTURE_ARRIVE").is_some_and(|v| v == "1"))
}

/// The soft line in percent of the card's budget (`VICTORY_TEXTURE_HOLD_PERCENT`, 50 to 85, 80 by
/// default); the hard line is always 88.
fn percents() -> (u64, u64) {
    static SOFT: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    let soft = *SOFT.get_or_init(|| {
        std::env::var("VICTORY_TEXTURE_HOLD_PERCENT")
            .ok()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .map_or(80, |p| p.clamp(50, 85))
    });
    (soft, 88)
}

/// Trimmed past the soft line beyond what it takes to get under it.
const MARGIN: u64 = 256 << 20;

/// What the sweep does with an idle copy the title still holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Level {
    /// Under the soft line: kept.
    Keep,
    /// Between the lines: the least recently used go until this many bytes are free.
    Trim(u64),
    /// Past the hard line, or the card unknown: let go as before.
    Sweep,
}

/// The level for the card's `[budget, used, ..]` with the soft and hard percents.
pub(crate) fn level(card: Option<[u64; 4]>, (soft, hard): (u64, u64)) -> Level {
    let Some([budget, used, ..]) = card else {
        return Level::Sweep;
    };
    if budget == 0 {
        return Level::Sweep;
    }
    let (soft, hard) = (budget / 100 * soft, budget / 100 * hard);
    if used >= hard {
        Level::Sweep
    } else if used >= soft {
        Level::Trim(used - soft + MARGIN)
    } else {
        Level::Keep
    }
}

/// The card's level now.
pub(crate) fn level_now() -> Level {
    level(crate::host::host().and_then(|h| h.video_memory()), percents())
}

/// Whether an idle copy is held: the switch is on, the card is not past the hard line, and the
/// title still holds the texture (placed, not placed over, its first page mapped).
pub(crate) fn held(on: bool, level: Level, placed: bool, replaced: bool, mapped: bool) -> bool {
    on && level != Level::Sweep && placed && !replaced && mapped
}

/// Of held copies `(last used, bytes, key)`, the least recently used that free `over` bytes.
pub(crate) fn least_used(mut held: Vec<(u64, u64, usize)>, over: u64) -> Vec<usize> {
    held.sort_by_key(|&(used, _, _)| used);
    let mut freed = 0u64;
    let mut out = Vec::new();
    for (_, bytes, key) in held {
        if freed >= over {
            break;
        }
        freed += bytes;
        out.push(key);
    }
    out
}

/// Since the line: idle copies held (each sweep that kept one), let go as least used, let go past
/// the hard line or because the title no longer holds them; made again with their memory as it was at the let-go (count,
/// bytes), made again with it changed (count, bytes); textures handed over as their bytes landed,
/// landings that found a texture with a copy; idle copies swept under the hard line because the
/// texture is not placed, was placed over, or its first page was unmapped; made again unchanged
/// but unmapped at the let-go (zeros both times, not counted as unchanged).
pub(crate) static STATS: [AtomicU64; 13] = [const { AtomicU64::new(0) }; 13];

/// What the memory looked like when a copy was let go, by texture.
#[cfg(feature = "diagnostics")]
static PRINTS: Mutex<Option<HashMap<usize, (u64, bool)>>> = Mutex::new(None);

/// A copy of texture `key` is let go: its memory's fingerprint is noted.
#[cfg(feature = "diagnostics")]
pub(crate) fn let_go(key: usize, at: u64, len: u64) {
    let print = (crate::texture::fingerprint(at, len), crate::texture::committed(at as *const u8, 1));
    let mut prints = PRINTS.lock().unwrap_or_else(|e| e.into_inner());
    let prints = prints.get_or_insert_with(HashMap::new);
    if prints.len() >= 1 << 18 {
        prints.clear();
    }
    prints.insert(key, print);
}

/// A copy of texture `key` let go before is made again: counted by whether its memory changed.
#[cfg(feature = "diagnostics")]
pub(crate) fn made_again(key: usize, at: u64, len: u64, bytes: u64) {
    let Some(was) = PRINTS.lock().unwrap_or_else(|e| e.into_inner()).as_mut().and_then(|p| p.remove(&key)) else {
        return;
    };
    let same = crate::texture::fingerprint(at, len) == was.0;
    if same && !was.1 {
        STATS[12].fetch_add(1, Ordering::Relaxed);
        return;
    }
    let i = if same { 3 } else { 5 };
    STATS[i].fetch_add(1, Ordering::Relaxed);
    STATS[i + 1].fetch_add(bytes, Ordering::Relaxed);
}

/// DirectStorage wrote `[at, at + len)` of title memory: placed textures with a copy there go to
/// the worker to be read again now, not when a slot showing them is next looked at.
pub(crate) fn landed(at: u64, len: u64) {
    if !arrive() {
        return;
    }
    let found = crate::resource::placed_with_copies(at, len);
    if !found.is_empty() {
        STATS[8].fetch_add(1, Ordering::Relaxed);
    }
    for (resource, texture) in found {
        if crate::texture_worker::arrived(resource, texture) {
            STATS[7].fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Whether the card's room rule counts the render targets as made once they stop growing
/// (`VICTORY_ROOM_SETTLED=1`): the room kept for targets and buffer copies not made yet is then
/// a margin, not the 3 GB and 1 GB of the world's first frame.
pub(crate) fn room_settled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_ROOM_SETTLED").is_some_and(|v| v == "1"))
}

/// Kept for targets once settled.
const SETTLED_MARGIN: u64 = 512 << 20;

/// Targets that grew less than this in [`SETTLE_MS`] have settled.
const SETTLE_GROWTH: u64 = 64 << 20;
const SETTLE_MS: u64 = 20_000;

/// The room kept: as before, or with the switch and settled targets only the margin.
pub(crate) fn room_kept(settled: bool, buffers: u64, targets: u64) -> u64 {
    if settled { SETTLED_MARGIN } else { buffers + targets }
}

/// Whether the render targets (`targets` bytes now) have settled, with the switch on.
pub(crate) fn settled(targets: u64) -> bool {
    static SEEN: std::sync::Mutex<(u64, Option<std::time::Instant>)> = std::sync::Mutex::new((0, None));
    if !room_settled() {
        return false;
    }
    let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    let now = std::time::Instant::now();
    match seen.1 {
        Some(since) if targets < seen.0 + SETTLE_GROWTH => now.duration_since(since).as_millis() as u64 >= SETTLE_MS,
        _ => {
            *seen = (targets, Some(now));
            false
        }
    }
}

/// The terms of the last refusal of room: the card's use, room kept for buffer copies, for
/// targets, the line, and what was kept in the end; and refusals since the line.
pub(crate) static ROOM: [AtomicU64; 6] = [const { AtomicU64::new(0) }; 6];

pub(crate) fn refused(terms: [u64; 5]) {
    for (i, t) in terms.into_iter().enumerate() {
        ROOM[i].store(t, Ordering::Relaxed);
    }
    ROOM[5].fetch_add(1, Ordering::Relaxed);
}

/// Whether `[a, a + n)` and `[b, b + m)` share a byte.
pub(crate) fn overlap(a: u64, n: u64, b: u64, m: u64) -> bool {
    a < b.saturating_add(m) && b < a.saturating_add(n)
}

#[cfg(test)]
mod tests {
    use super::{Level, held, least_used, level, overlap};

    const GB: u64 = 1 << 30;

    #[test]
    fn the_card_level_follows_the_soft_and_hard_lines() {
        let card = |used| Some([10 * GB, used, 0, 0]);
        assert_eq!(level(card(7 * GB), (80, 88)), Level::Keep);
        assert_eq!(level(card(8 * GB + GB / 2), (80, 88)), Level::Trim(8 * GB + GB / 2 - 10 * GB / 100 * 80 + (256 << 20)));
        assert_eq!(level(card(9 * GB), (80, 88)), Level::Sweep);
        assert_eq!(level(None, (80, 88)), Level::Sweep, "an unknown card is swept as before");
    }

    #[test]
    fn an_idle_copy_the_title_holds_is_kept_only_with_the_switch() {
        assert!(!held(false, Level::Keep, true, false, true), "off: let go as before");
        assert!(held(true, Level::Keep, true, false, true));
        assert!(held(true, Level::Trim(1), true, false, true), "between the lines: held, then trimmed");
        assert!(!held(true, Level::Sweep, true, false, true), "past the hard line: as before");
        assert!(!held(true, Level::Keep, true, true, true), "placed over: the title let it go");
        assert!(!held(true, Level::Keep, true, false, false), "unmapped: the title let it go");
        assert!(!held(true, Level::Keep, false, false, true), "only placed textures");
    }

    #[test]
    fn the_least_recently_used_go_first_until_enough_is_free() {
        let held = vec![(30, 100, 3), (10, 100, 1), (20, 100, 2), (40, 100, 4)];
        assert_eq!(least_used(held.clone(), 150), [1, 2]);
        assert_eq!(least_used(held.clone(), 0), Vec::<usize>::new());
        assert_eq!(least_used(held, 1000), [1, 2, 3, 4]);
    }

    #[test]
    fn settled_targets_keep_only_a_margin() {
        assert_eq!(super::room_kept(false, 362 << 20, 1803 << 20), 2165 << 20, "as before");
        assert_eq!(super::room_kept(true, 362 << 20, 1803 << 20), 512 << 20);
    }

    #[test]
    fn spans_overlap_only_when_they_share_a_byte() {
        assert!(overlap(0, 10, 9, 1));
        assert!(!overlap(0, 10, 10, 5));
        assert!(overlap(5, 1, 0, 10));
        assert!(!overlap(0, 0, 0, 10));
    }
}
