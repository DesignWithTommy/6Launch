//! VICTORY_WS_MEMO=1: the heap pass asks the kernel how much of a texture is mapped once per texture per frame

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

pub(crate) fn on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_WS_MEMO").is_some_and(|v| v == "1"))
}

// answers by texture start: (end, mapped bytes), for one frame
#[derive(Default)]
struct Memo {
    frame: u64,
    // moves on every remap, so an answer asked across one is not kept
    remaps: u64,
    longest: usize,
    answers: BTreeMap<usize, (usize, u64)>,
}

impl Memo {
    fn get(&mut self, at: usize, len: usize, frame: u64) -> Option<u64> {
        if self.frame != frame {
            self.answers.clear();
            self.longest = 0;
            self.frame = frame;
        }
        self.answers.get(&at).filter(|a| a.0 == at.saturating_add(len)).map(|a| a.1)
    }

    fn put(&mut self, at: usize, len: usize, frame: u64, remaps: u64, mapped: u64) {
        if self.frame != frame || self.remaps != remaps {
            return;
        }
        self.longest = self.longest.max(len);
        self.answers.insert(at, (at.saturating_add(len), mapped));
    }

    fn remapped(&mut self, at: usize, len: usize) {
        self.remaps += 1;
        if self.answers.is_empty() {
            return;
        }
        let end = at.saturating_add(len);
        let gone: Vec<usize> = self
            .answers
            .range(at.saturating_sub(self.longest)..end)
            .filter(|(_, a)| a.0 > at)
            .map(|(k, _)| *k)
            .collect();
        for k in gone {
            self.answers.remove(&k);
        }
    }
}

static MEMO: Mutex<Option<Memo>> = Mutex::new(None);
static COUNTS: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];

fn memo() -> std::sync::MutexGuard<'static, Option<Memo>> {
    MEMO.lock().unwrap_or_else(|e| e.into_inner())
}

/// `crate::texture::mapped_bytes`, remembered for the frame.
pub(crate) fn mapped_bytes(at: *const u8, len: usize, frame: u64) -> u64 {
    if !on() {
        return crate::texture::mapped_bytes(at, len);
    }
    let remaps = {
        let mut m = memo();
        let m = m.get_or_insert_with(Memo::default);
        if let Some(mapped) = m.get(at as usize, len, frame) {
            let n = COUNTS[0].fetch_add(1, Ordering::Relaxed);
            if n.is_multiple_of(100_000) {
                tracing::info!(hits = n + 1, asked = COUNTS[1].load(Ordering::Relaxed), "working-set memo");
            }
            return mapped;
        }
        m.remaps
    };
    COUNTS[1].fetch_add(1, Ordering::Relaxed);
    let mapped = crate::texture::mapped_bytes(at, len);
    if let Some(m) = memo().as_mut() {
        m.put(at as usize, len, frame, remaps, mapped);
    }
    mapped
}

/// Title memory `[at, at + len)` was mapped or unmapped: answers over it are dropped. Locks only its own mutex.
pub(crate) fn remapped(at: usize, len: usize) {
    if !on() {
        return;
    }
    if let Some(m) = memo().as_mut() {
        m.remapped(at, len);
    }
}

#[cfg(test)]
mod tests {
    use super::Memo;

    #[test]
    fn an_answer_is_kept_for_its_frame_only() {
        let mut m = Memo::default();
        assert_eq!(m.get(0x1000, 0x4000, 7), None);
        m.put(0x1000, 0x4000, 7, 0, 0x2000);
        assert_eq!(m.get(0x1000, 0x4000, 7), Some(0x2000));
        assert_eq!(m.get(0x1000, 0x8000, 7), None);
        assert_eq!(m.get(0x1000, 0x4000, 8), None);
    }

    #[test]
    fn a_remap_over_a_texture_drops_its_answer() {
        let mut m = Memo::default();
        m.get(0, 1, 3);
        m.put(0x10000, 0x20000, 3, 0, 0x10000);
        m.put(0x40000, 0x10000, 3, 0, 0x10000);
        m.remapped(0x28000, 0x1000);
        assert_eq!(m.get(0x10000, 0x20000, 3), None);
        assert_eq!(m.get(0x40000, 0x10000, 3), Some(0x10000));
        m.remapped(0x50000, 0x1000);
        assert_eq!(m.get(0x40000, 0x10000, 3), Some(0x10000));
        m.remapped(0x3f000, 0x1001);
        assert_eq!(m.get(0x40000, 0x10000, 3), None);
    }

    #[test]
    fn an_answer_asked_across_a_remap_is_not_kept() {
        let mut m = Memo::default();
        m.get(0, 1, 5);
        let remaps = m.remaps;
        m.remapped(0x900000, 0x1000);
        m.put(0x10000, 0x1000, 5, remaps, 0x1000);
        assert_eq!(m.get(0x10000, 0x1000, 5), None);
    }
}
