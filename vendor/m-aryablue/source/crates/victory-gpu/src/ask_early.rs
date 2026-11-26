//! a flush's askings about absent blocks, made ahead on a helper thread (VICTORY_ASK_EARLY=1, with VICTORY_WIDE_GUARDS=1)

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

const BLOCK: usize = crate::watch::PAGE;
// an answer older than this many frames is asked again by the flush itself
const FRAMES: u64 = 2;
const LOG_MOST: usize = 1 << 12;
const WANT_MOST: usize = 1 << 16;

fn switch(name: &str) -> bool {
    std::env::var_os(name).is_some_and(|v| v == "1")
}

pub(crate) fn on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        let on = switch("VICTORY_ASK_EARLY");
        if on {
            tracing::info!(
                check = check(),
                "absent blocks are asked about ahead of the next flush, on a helper thread (VICTORY_ASK_EARLY=1)"
            );
        }
        on
    })
}

// each answer taken is asked again and compared (also with burst refresh's check switches)
fn check() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        ["VICTORY_ASK_EARLY_CHECK", "VICTORY_BURST_REFRESH_CHECK", "VICTORY_BURST_GROUP_CHECK"]
            .iter()
            .any(|n| switch(n))
    })
}

// an asking under way: what was written, remapped or committed since it began
struct Pending {
    frame: u64,
    log: Vec<(usize, usize)>,
    all: bool,
}

#[derive(Default)]
struct State {
    // uncommitted stretches: start block to (end block, frame asked)
    ends: BTreeMap<usize, (usize, u64)>,
    // the longest stretch kept, in blocks
    longest: usize,
    pending: Option<Pending>,
    // absent blocks the flushes met since the last asking began
    want: BTreeSet<usize>,
}

impl State {
    // memory `[at, end)` was written, remapped or committed: answers over it stand no longer
    fn note(&mut self, at: usize, end: usize) {
        let (a, b) = (at / BLOCK, end.div_ceil(BLOCK));
        if !self.ends.is_empty() {
            let over: Vec<usize> = self
                .ends
                .range(a.saturating_sub(self.longest)..b)
                .filter(|&(_, &(to, _))| to > a)
                .map(|(&from, _)| from)
                .collect();
            for from in over {
                self.ends.remove(&from);
            }
        }
        if let Some(p) = &mut self.pending
            && !p.all
        {
            if p.log.len() >= LOG_MOST {
                p.log.clear();
                p.all = true;
            } else {
                p.log.push((a, b));
            }
        }
    }

    // the blocks to ask about next, if no asking is under way
    fn begin(&mut self, frame: u64) -> Option<Vec<usize>> {
        if self.pending.is_some() || self.want.is_empty() {
            return None;
        }
        self.pending = Some(Pending {
            frame,
            log: Vec::new(),
            all: false,
        });
        Some(std::mem::take(&mut self.want).into_iter().collect())
    }

    // the asking's answers; those written over since it began are dropped
    fn finish(&mut self, found: Vec<(usize, usize)>) {
        let Some(p) = self.pending.take() else {
            return;
        };
        if p.all {
            return;
        }
        for (from, to) in found {
            if !p.log.iter().any(|&(a, b)| a < to && from < b) {
                self.longest = self.longest.max(to - from);
                self.ends.insert(from, (to, p.frame));
            }
        }
    }

    // where the uncommitted memory at `block` ends, if a fresh answer says
    fn end_of(&self, block: usize, frame: u64) -> Option<usize> {
        let (&from, &(to, asked)) = self.ends.range(..=block).next_back()?;
        (from <= block && block < to && frame.saturating_sub(asked) <= FRAMES).then_some(to)
    }

    // of a flush's absent `blocks` (in order): the stretches answered, and the blocks left to ask
    fn take(&mut self, blocks: &[usize], frame: u64, found: &mut BTreeMap<usize, usize>) -> Vec<usize> {
        let mut left = Vec::new();
        let mut until = 0;
        for &block in blocks {
            if self.want.len() < WANT_MOST {
                self.want.insert(block);
            }
            if block < until {
                continue;
            }
            match self.end_of(block, frame) {
                Some(to) => {
                    found.insert(block, to);
                    until = to;
                }
                None => left.push(block),
            }
        }
        left
    }
}

static STATE: Mutex<State> = Mutex::new(State {
    ends: BTreeMap::new(),
    longest: 0,
    pending: None,
    want: BTreeSet::new(),
});

fn state() -> std::sync::MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

// flushes, stretches taken from early answers, blocks asked by the flush, answers the check found wrong
static STATS: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];

// title memory `[at, at + len)` was written, remapped or committed
pub(crate) fn wrote(at: usize, len: usize) {
    if len == 0 || !on() {
        return;
    }
    state().note(at, at.saturating_add(len));
}

// a flush over `absent` (each range's absent blocks): the early answers, and the lists it must ask itself
pub(crate) fn take(absent: Vec<Vec<usize>>) -> (BTreeMap<usize, usize>, Vec<Vec<usize>>) {
    let frame = crate::frame::current();
    let mut found = BTreeMap::new();
    let left: Vec<Vec<usize>> = {
        let mut s = state();
        absent
            .iter()
            .map(|blocks| s.take(blocks, frame, &mut found))
            .filter(|v| !v.is_empty())
            .collect()
    };
    if check() {
        found.retain(|&block, to| {
            let now = crate::watch::uncommitted_end_of(block);
            // longer now is no fault: the stretch was asked from where it began then
            if now >= *to {
                return true;
            }
            STATS[3].fetch_add(1, Relaxed);
            tracing::warn!(
                block = format_args!("{:#x}", block * BLOCK),
                early = *to - block,
                now = now.saturating_sub(block),
                "an early answer about uncommitted blocks no longer holds at the flush"
            );
            *to = now;
            now > block
        });
    }
    STATS[0].fetch_add(1, Relaxed);
    STATS[1].fetch_add(found.len() as u64, Relaxed);
    STATS[2].fetch_add(left.iter().map(|v| v.len() as u64).sum(), Relaxed);
    line();
    (found, left)
}

// a flush ended: the blocks its like will need are asked about on the helper
pub(crate) fn next() {
    let Some(blocks) = state().begin(crate::frame::current()) else {
        return;
    };
    static HELPER: std::sync::OnceLock<Option<std::sync::mpsc::Sender<Vec<usize>>>> = std::sync::OnceLock::new();
    let helper = HELPER.get_or_init(|| {
        let (send, jobs) = std::sync::mpsc::channel::<Vec<usize>>();
        std::thread::Builder::new()
            .name("victory-ask-early".into())
            .spawn(move || {
                for blocks in jobs {
                    let mut found = Vec::new();
                    let mut until = 0;
                    for block in blocks {
                        if block < until {
                            continue;
                        }
                        until = crate::watch::uncommitted_end_of(block);
                        if until > block {
                            found.push((block, until));
                        }
                    }
                    state().finish(found);
                }
            })
            .ok()
            .map(|_| send)
    });
    if helper.as_ref().is_none_or(|h| h.send(blocks).is_err()) {
        state().finish(Vec::new());
    }
}

fn line() {
    const EVERY: u64 = 4096;
    if STATS[0].load(Relaxed) < EVERY {
        return;
    }
    let [flushes, early, asked, wrong] = [0, 1, 2, 3].map(|i| STATS[i].swap(0, Relaxed));
    let per = flushes.max(1) as f64;
    tracing::info!(
        "early askings per flush (VICTORY_ASK_EARLY=1): {:.2} uncommitted stretches answered ahead, {:.2} blocks asked by the flush; \
         the check found {wrong} answers that no longer held, in {flushes} flushes",
        early as f64 / per,
        asked as f64 / per,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    const B: usize = BLOCK;

    // an asking about `want` that finds `found`, with `between` run while it is under way
    fn asked(s: &mut State, want: &[usize], found: &[(usize, usize)], frame: u64, between: impl FnOnce(&mut State)) {
        let mut none = BTreeMap::new();
        s.take(want, frame, &mut none);
        assert_eq!(s.begin(frame), Some(want.to_vec()));
        between(s);
        s.finish(found.to_vec());
    }

    #[test]
    fn a_flush_takes_what_was_asked_ahead_and_asks_the_rest() {
        let mut s = State::default();
        asked(&mut s, &[10, 11, 12, 40], &[(10, 13)], 7, |_| {});
        let mut found = BTreeMap::new();
        assert_eq!(s.take(&[10, 11, 12, 40, 50], 8, &mut found), vec![40, 50]);
        assert_eq!(found, BTreeMap::from([(10, 13)]));
        // a block inside the stretch is answered too
        assert_eq!(s.end_of(12, 8), Some(13));
        assert_eq!(s.end_of(13, 8), None);
    }

    #[test]
    fn a_write_between_the_early_asking_and_the_flush_is_asked_again() {
        let mut s = State::default();
        asked(&mut s, &[10, 20], &[(10, 14), (20, 22)], 7, |_| {});
        s.note(11 * B + 5, 11 * B + 6);
        let mut found = BTreeMap::new();
        assert_eq!(s.take(&[10, 20], 7, &mut found), vec![10]);
        assert_eq!(found, BTreeMap::from([(20, 22)]));
    }

    #[test]
    fn a_remap_while_the_asking_is_under_way_drops_its_answer_there() {
        let mut s = State::default();
        asked(&mut s, &[10, 20], &[(10, 14), (20, 22)], 7, |s| s.note(21 * B, 30 * B));
        let mut found = BTreeMap::new();
        assert_eq!(s.take(&[10, 20], 7, &mut found), vec![20]);
        assert_eq!(found, BTreeMap::from([(10, 14)]));
    }

    #[test]
    fn a_commit_ends_the_answers_over_it_before_and_after_they_land() {
        let mut s = State::default();
        // the old answer over the commit goes at once, the one under way when it lands
        asked(&mut s, &[10], &[(10, 14)], 7, |_| {});
        asked(&mut s, &[10, 30], &[(10, 14), (30, 31)], 7, |s| {
            s.note(13 * B, 14 * B);
            assert_eq!(s.end_of(10, 7), None);
        });
        assert_eq!(s.end_of(10, 7), None);
        assert_eq!(s.end_of(30, 7), Some(31));
        // a commit beside a stretch leaves it
        s.note(31 * B, 32 * B);
        assert_eq!(s.end_of(30, 7), Some(31));
    }

    #[test]
    fn a_full_log_drops_the_whole_asking() {
        let mut s = State::default();
        asked(&mut s, &[10, 20], &[(10, 14), (20, 22)], 7, |s| {
            for i in 0..=LOG_MOST {
                s.note((1000 + i) * B, (1001 + i) * B);
            }
        });
        let mut found = BTreeMap::new();
        assert_eq!(s.take(&[10, 20], 7, &mut found), vec![10, 20]);
        assert!(found.is_empty());
        // the next asking stands again
        asked(&mut s, &[10, 20], &[(10, 14)], 7, |_| {});
        assert_eq!(s.end_of(11, 7), Some(14));
    }

    #[test]
    fn an_old_answer_is_asked_again_and_one_asking_runs_at_a_time() {
        let mut s = State::default();
        asked(&mut s, &[10], &[(10, 14)], 7, |s| {
            let mut none = BTreeMap::new();
            s.take(&[50], 7, &mut none);
            assert_eq!(s.begin(7), None);
        });
        assert_eq!(s.end_of(10, 7 + FRAMES), Some(14));
        assert_eq!(s.end_of(10, 8 + FRAMES), None);
        assert_eq!(s.begin(9), Some(vec![50]));
    }
}
