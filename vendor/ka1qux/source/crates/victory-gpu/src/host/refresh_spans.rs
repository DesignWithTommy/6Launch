//! A submission's refresh steps found by an index (`VICTORY_STEP_INDEX`; off
//! by default), a submodule of `host`: what `stale_under_flights` gathers.

use super::*;

/// Whether a submission's refresh steps find their copy by an index instead
/// of a search (`VICTORY_STEP_INDEX=1`; off by default; `alt`: twenty seconds
/// with, twenty without, in turn, each change said in the log). The same
/// copies and spans come out ([`RefreshSpans`], which a unit test holds to the
/// search); what goes is a lock and a search of the spans per refresh step.
/// (After fxceless/victory-fork 84c5dc00, Steenax6/VIctory-NVID; the other
/// half of that change, the ranges given to shaders found by an index, is in
/// our tree already as `WriteGroups`.)
pub(super) fn step_index() -> bool {
    /// 0 off, 1 on, 2 in turn.
    static MODE: OnceLock<u8> = OnceLock::new();
    match *MODE.get_or_init(|| match std::env::var("VICTORY_STEP_INDEX").as_deref() {
        Ok("1") => 1,
        Ok("alt") => 2,
        _ => 0,
    }) {
        0 => false,
        1 => {
            static SAID: std::sync::Once = std::sync::Once::new();
            SAID.call_once(|| {
                tracing::info!("a submission's refresh steps find their copy by an index (VICTORY_STEP_INDEX=1)");
            });
            true
        }
        _ => {
            static EPOCH: OnceLock<std::time::Instant> = OnceLock::new();
            let on = (EPOCH.get_or_init(std::time::Instant::now).elapsed().as_secs() / 20) % 2 == 1;
            static LAST: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(2);
            if LAST.swap(u8::from(on), Ordering::Relaxed) != u8::from(on) {
                tracing::info!(
                    "a submission's refresh steps find their copy by an index from now (VICTORY_STEP_INDEX=alt): {}",
                    if on { "ON" } else { "off, by a search as usual" }
                );
            }
            on
        }
    }
}

/// The spans refreshed per host copy that has writes pending
/// ([`stale_under_flights`]): the copies in the order their first step comes,
/// each with the lowest and highest address its steps reach.
///
/// Searched, each step asks its copy whether writes are pending (a lock) and
/// looks for the copy among the spans so far: 20,000 to 43,000 refresh steps a
/// frame in the tree the index comes from. Indexed, a copy is asked once, by
/// its address (steps of one copy mostly follow one another, so the last
/// answer is tried first). Only the submitting thread adds pending writes,
/// after this; a landing may take a copy's away meanwhile, and its span is
/// then as if the landing came before or after all its steps, which the merge
/// that follows finds nothing pending for.
pub(super) struct RefreshSpans<T> {
    pub(super) spans: Vec<(std::sync::Arc<T>, usize, usize)>,
    /// With the index: by copy, its place in `spans`, or that nothing is
    /// pending on it.
    known: Option<std::collections::HashMap<usize, Option<usize>>>,
    last: (usize, Option<usize>),
}

impl<T> RefreshSpans<T> {
    pub(super) fn new(indexed: bool) -> Self {
        Self { spans: Vec::new(), known: indexed.then(std::collections::HashMap::new), last: (0, None) }
    }

    /// A refresh of `[address, address + len)` of `shadow`; `pending` says
    /// whether writes are pending on a copy.
    pub(super) fn add(&mut self, shadow: &std::sync::Arc<T>, address: usize, len: usize, pending: impl Fn(&T) -> bool) {
        let Some(known) = &mut self.known else {
            if pending(shadow) {
                match self.spans.iter_mut().find(|s| std::sync::Arc::ptr_eq(&s.0, shadow)) {
                    Some(s) => (s.1, s.2) = (s.1.min(address), s.2.max(address + len)),
                    None => self.spans.push((shadow.clone(), address, address + len)),
                }
            }
            return;
        };
        let key = std::sync::Arc::as_ptr(shadow) as *const () as usize;
        let at = if self.last.0 == key {
            self.last.1
        } else {
            let at = *known.entry(key).or_insert_with(|| {
                pending(shadow).then(|| {
                    self.spans.push((shadow.clone(), usize::MAX, 0));
                    self.spans.len() - 1
                })
            });
            self.last = (key, at);
            at
        };
        if let Some(at) = at {
            let s = &mut self.spans[at];
            (s.1, s.2) = (s.1.min(address), s.2.max(address + len));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::RefreshSpans;
    use std::sync::Arc;

    /// The index gives what the search gives: the same copies in the same
    /// order, each with the same span, whatever copies have writes pending
    /// and however the steps of the copies take turns.
    #[test]
    fn the_index_gives_what_the_search_gives() {
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = |below: usize| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 33) as usize % below
        };
        for (copies, steps) in [(1, 5), (3, 200), (40, 3000), (500, 3000)] {
            for _ in 0..20 {
                let shadows: Vec<Arc<usize>> = (0..copies).map(Arc::new).collect();
                let pending_mod = 1 + next(3);
                let pending = |n: &usize| n.is_multiple_of(pending_mod);
                let given: Vec<(usize, usize, usize)> =
                    (0..steps).map(|_| (next(copies), next(64) * 64, 16 + next(4) * 16)).collect();
                let spans_by = |indexed: bool| {
                    let mut spans = RefreshSpans::new(indexed);
                    for &(copy, address, len) in &given {
                        spans.add(&shadows[copy], address, len, pending);
                    }
                    spans.spans.into_iter().map(|(s, from, to)| (*s, from, to)).collect::<Vec<_>>()
                };
                assert_eq!(spans_by(true), spans_by(false), "{copies} copies, {steps} steps");
            }
        }
    }
}
