//! Speed switches read live, so that one run measures both sides of them.
//!
//! `VICTORY_ALT=<NAME>[,<NAME>...]` (with or without `VICTORY_`): those
//! switches go on and off together every `VICTORY_ALT_SECS` seconds (20),
//! at the end of a title present, starting off; the frame lines carry
//! `[alt=on]` or `[alt=off]`. Only the names in [`LIVE`] can flip.

use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering::Relaxed};
use std::sync::OnceLock;

/// The switches made safe to flip mid-run.
pub(crate) const LIVE: [&str; 12] = [
    "TABLE_REUSE",
    "LOOKUP_FAST",
    "HEAP_SYNC_AHEAD",
    "COPY_STREAM",
    "JOURNAL_FLAT",
    "PIECES_FLAT",
    "FAULT_LOG",
    "COMMIT_NOTICE",
    "COPY_FLUSH_OVERLAP",
    "WIDE_LAND",
    "MERGE_WIDE",
    "MADE_FAST",
];

// unread, off, on, flipped by alt
const UNREAD: u8 = 0;
const OFF: u8 = 1;
const ON: u8 = 2;
const ALT: u8 = 3;

/// A `VICTORY_<NAME>=1` switch, read once from the environment and then
/// from an atomic; one named in `VICTORY_ALT` follows the alt phase.
pub(crate) struct Switch {
    name: &'static str,
    state: AtomicU8,
    // logged at the first read when it can be on
    note: Option<&'static str>,
}

impl Switch {
    /// `name` without `VICTORY_`.
    pub(crate) const fn new(name: &'static str) -> Self {
        Switch { name, state: AtomicU8::new(UNREAD), note: None }
    }

    /// With a line said at the first read if it can be on.
    pub(crate) const fn noted(name: &'static str, note: &'static str) -> Self {
        Switch { name, state: AtomicU8::new(UNREAD), note: Some(note) }
    }

    pub(crate) fn on(&self) -> bool {
        match self.state.load(Relaxed) {
            OFF => false,
            ON => true,
            ALT => ALT_ON.load(Relaxed),
            _ => self.read(),
        }
    }

    /// Whether it can be on at some point of this run.
    pub(crate) fn ever(&self) -> bool {
        self.on();
        self.state.load(Relaxed) != OFF
    }

    #[cold]
    fn read(&self) -> bool {
        let state = if names().iter().any(|n| n == self.name) {
            ALT
        } else if std::env::var_os(format!("VICTORY_{}", self.name)).is_some_and(|v| v == "1") {
            ON
        } else {
            OFF
        };
        if self.state.swap(state, Relaxed) == UNREAD
            && state != OFF
            && let Some(note) = self.note
        {
            tracing::info!("{note}{}", if state == ALT { " (VICTORY_ALT: in turn)" } else { "" });
        }
        self.on()
    }

    #[cfg(test)]
    pub(crate) fn set_for_test(&self, on: Option<bool>) {
        self.state.store(
            match on {
                Some(true) => ON,
                Some(false) => OFF,
                None => ALT,
            },
            Relaxed,
        );
    }
}

static ALT_ON: AtomicBool = AtomicBool::new(false);
static FLIPS: AtomicU64 = AtomicU64::new(0);

/// The names in `VICTORY_ALT`, without `VICTORY_`, those that cannot flip
/// left out (and said so).
fn names() -> &'static [String] {
    static NAMES: OnceLock<Vec<String>> = OnceLock::new();
    NAMES.get_or_init(|| {
        let list = std::env::var("VICTORY_ALT").unwrap_or_default();
        let (live, dead) = parse(&list);
        if !dead.is_empty() {
            tracing::warn!(
                "VICTORY_ALT: {} cannot flip inside a run and stay as set (the switches that can: {})",
                dead.join(", "),
                LIVE.join(", ")
            );
        }
        if !live.is_empty() {
            tracing::warn!(
                "VICTORY_ALT: {} go on and off together every {} s at a present, off first; frame lines say [alt=on] or [alt=off]",
                live.join(", "),
                secs()
            );
        }
        live
    })
}

// the live names in `list` and the others
fn parse(list: &str) -> (Vec<String>, Vec<String>) {
    list.split(',')
        .map(|n| n.trim().trim_start_matches("VICTORY_").to_ascii_uppercase())
        .filter(|n| !n.is_empty())
        .partition(|n| LIVE.contains(&n.as_str()))
}

fn secs() -> u64 {
    static SECS: OnceLock<u64> = OnceLock::new();
    *SECS.get_or_init(|| std::env::var("VICTORY_ALT_SECS").ok().and_then(|v| v.trim().parse().ok()).unwrap_or(20).max(1))
}

/// Whether the run alternates any switch.
pub(crate) fn alternating() -> bool {
    !names().is_empty()
}

/// Times the alt phase has changed.
pub(crate) fn flips() -> u64 {
    FLIPS.load(Relaxed)
}

/// The tag a frame line carries: ` [alt=on]`, ` [alt=off]`, or nothing.
pub(crate) fn tag() -> &'static str {
    match (alternating(), ALT_ON.load(Relaxed)) {
        (false, _) => "",
        (true, true) => " [alt=on]",
        (true, false) => " [alt=off]",
    }
}

/// The tag of the phase before the last change.
pub(crate) fn tag_before() -> &'static str {
    match (alternating(), ALT_ON.load(Relaxed)) {
        (false, _) => "",
        (true, true) => " [alt=off]",
        (true, false) => " [alt=on]",
    }
}

/// The end of a title present: the phase changes once its time is up.
pub fn frame_end() {
    if !alternating() {
        return;
    }
    static SINCE: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);
    let mut since = SINCE.lock().unwrap_or_else(|e| e.into_inner());
    let now = std::time::Instant::now();
    let start = *since.get_or_insert(now);
    if now.duration_since(start).as_secs() >= secs() {
        *since = Some(now);
        flip();
    }
}

fn flip() {
    let on = !ALT_ON.load(Relaxed);
    ALT_ON.store(on, Relaxed);
    FLIPS.fetch_add(1, Relaxed);
    tracing::warn!(
        target: "victory_gpu::host::frame",
        "alt: {} {} from now",
        names().join(", "),
        if on { "ON" } else { "off" }
    );
}

#[cfg(test)]
pub(crate) fn flip_for_test() {
    flip();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alt_names_take_live_switches_only() {
        let (live, dead) = parse("VICTORY_TABLE_REUSE, copy_stream,INPLACE,,MADE_FAST");
        assert_eq!(live, ["TABLE_REUSE", "COPY_STREAM", "MADE_FAST"]);
        assert_eq!(dead, ["INPLACE"]);
    }

    #[test]
    fn an_alternating_switch_follows_the_phase() {
        static S: Switch = Switch::new("TEST_ONLY");
        S.set_for_test(None);
        let first = S.on();
        flip_for_test();
        assert_eq!(S.on(), !first);
        flip_for_test();
        assert_eq!(S.on(), first);
        assert!(S.ever());
        S.set_for_test(Some(false));
        assert!(!S.on() && !S.ever());
    }
}
