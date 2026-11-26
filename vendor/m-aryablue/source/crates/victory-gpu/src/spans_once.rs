//! `VICTORY_SPANS_ONCE=1` (off by default): a submission's refresh steps
//! are walked once for the spans of both the one write-watch asking and
//! the look under flights (`crate::host::stale_under_flights`), not once
//! for each, and a copy's span is found by its key, not by a search along
//! the list. The spans, and their order for the look, are the same.

use std::collections::HashMap;

pub(crate) fn on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        let on = std::env::var_os("VICTORY_SPANS_ONCE").is_some_and(|v| v == "1");
        if on {
            tracing::info!("a submission's refresh spans are built once for the asking and the look under flights (VICTORY_SPANS_ONCE=1)");
        }
        on
    })
}

/// Per copy, the span of its refreshed ranges: for the asking (by key) and
/// for the look under flights (in the order the copies first come).
pub(crate) struct Spans<T> {
    pub(crate) ask: HashMap<usize, (usize, usize)>,
    pub(crate) stale: Vec<(T, usize, usize)>,
    // where each copy is in `stale`, by key
    at: HashMap<usize, usize>,
}

impl<T: Clone> Spans<T> {
    pub(crate) fn new() -> Self {
        Spans {
            ask: HashMap::new(),
            stale: Vec::new(),
            at: HashMap::new(),
        }
    }

    /// A step (copy `copy` by its key, `[address, address + len)`) that
    /// counts for the asking (`asked`) and for the look (`looked`).
    pub(crate) fn add(&mut self, copy: &T, k: usize, address: usize, len: usize, asked: bool, looked: bool) {
        let end = address + len;
        if asked {
            let span = self.ask.entry(k).or_insert((address, end));
            *span = (span.0.min(address), span.1.max(end));
        }
        if looked {
            match self.at.get(&k) {
                Some(&i) => {
                    let s = &mut self.stale[i];
                    (s.1, s.2) = (s.1.min(address), s.2.max(end));
                }
                None => {
                    self.at.insert(k, self.stale.len());
                    self.stale.push((copy.clone(), address, end));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The spans are those of the two walks as they were: the asking's by
    /// key, the look's in first-come order.
    #[test]
    fn one_walk_gives_both_walks_spans() {
        let mut x = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = move || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as usize
        };
        for case in 0..500 {
            let copies = 1 + next() % 40;
            let steps: Vec<(usize, usize, usize)> = (0..next() % 300)
                .map(|_| (next() % copies, next() % 100_000, 1 + next() % 5_000))
                .collect();
            let unwatched: Vec<bool> = (0..copies).map(|_| next() % 5 == 0).collect();
            let pending: Vec<bool> = (0..copies).map(|_| next() % 3 != 0).collect();
            let covered = |c: usize, a: usize| (c * 7 + a / 1000) % 4 == 0;
            let once = case % 2 == 0;
            // the asking's walk, as it was
            let mut ask_old: HashMap<usize, (usize, usize)> = HashMap::new();
            for &(c, a, n) in &steps {
                if !unwatched[c] && !covered(c, a) {
                    let span = ask_old.entry(c).or_insert((a, a + n));
                    *span = (span.0.min(a), span.1.max(a + n));
                }
            }
            // the look's walk, as it was
            let mut stale_old: Vec<(usize, usize, usize)> = Vec::new();
            for &(c, a, n) in &steps {
                if pending[c] && !(once && covered(c, a)) {
                    match stale_old.iter_mut().find(|s| s.0 == c) {
                        Some(s) => (s.1, s.2) = (s.1.min(a), s.2.max(a + n)),
                        None => stale_old.push((c, a, a + n)),
                    }
                }
            }
            let mut spans = Spans::new();
            for &(c, a, n) in &steps {
                spans.add(&c, c, a, n, !unwatched[c] && !covered(c, a), pending[c] && !(once && covered(c, a)));
            }
            assert_eq!(spans.ask, ask_old, "case {case}");
            assert_eq!(spans.stale, stale_old, "case {case}");
        }
    }
}
