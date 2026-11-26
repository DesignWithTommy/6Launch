//! A list's upload fills shared among threads (`VICTORY_FILL_WORKERS`; off by
//! default), a submodule of `host`: it works on the list's steps
//! ([`Step`]) and copies through `copy_mapped`.

use super::*;

// After upstream VIctory pull request #2900 (0xkn0wh3r3), from DexNom/VIctory
// 5f4c3df (`Fill`, `independent_fills`, `run_fills` and their tests), with the
// panic guard of that pull request's review.
/// Threads a run of a list's upload fills is shared among at submission
/// (`VICTORY_FILL_WORKERS`, 1 to 6; 1, one at a time, by default).
pub(super) fn fill_workers() -> usize {
    static COUNT: OnceLock<usize> = OnceLock::new();
    *COUNT.get_or_init(|| {
        let n = std::env::var("VICTORY_FILL_WORKERS")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(1)
            .clamp(1, 6);
        if n > 1 {
            tracing::info!(
                workers = n,
                "runs of a list's upload fills are shared among up to this many threads when the fills are independent \
                 (VICTORY_FILL_WORKERS)"
            );
        }
        n
    })
}

/// Since the last frame-rate line, with `VICTORY_FILL_WORKERS`: runs of fills
/// shared among threads, the fills in them, and the threads they used.
static FILL_STATS: [std::sync::atomic::AtomicU64; 3] = [const { std::sync::atomic::AtomicU64::new(0) }; 3];

/// For the frame-rate line: what [`run_fills`] shared, per frame.
pub(super) fn fill_line(frames: f64) {
    if fill_workers() < 2 {
        return;
    }
    let [runs, fills, threads] = [0, 1, 2].map(|i| FILL_STATS[i].swap(0, Ordering::Relaxed) as f64 / frames);
    tracing::info!(
        "upload fills shared among threads per frame (VICTORY_FILL_WORKERS={}): {runs:.1} runs, {fills:.0} fills, {:.1} threads a run",
        fill_workers(),
        if runs > 0.0 { threads / runs } else { 0.0 }
    );
}

/// A copy of title memory into a list's upload memory ([`Step::Fill`]).
#[derive(Clone, Copy)]
pub(super) struct Fill {
    pub(super) dst: usize,
    pub(super) src: usize,
    pub(super) len: usize,
}

impl Fill {
    fn run(self) {
        copy_mapped(self.dst as *mut u8, self.src as *const u8, self.len, false);
    }
}

/// The fills `steps` starts with, up to the first other step (which may
/// write title memory or the upload memory, and so ends the run).
pub(super) fn fill_run(steps: &[Step]) -> Vec<Fill> {
    steps
        .iter()
        .map_while(|s| match *s {
            Step::Fill(dst, src, len) => Some(Fill { dst, src, len }),
            _ => None,
        })
        .collect()
}

/// Threads that take shares of fill runs, started on first use (five, the
/// most [`fill_workers`] asks for besides the submitting thread); `None`
/// if none could be started (runs are done in order then). Threads spawned
/// for each run cost more than the copies (28,279 fills of 448 bytes: 17
/// ms on four threads made per run of 512, 10 ms on one).
fn fill_pool() -> Option<&'static std::sync::mpsc::Sender<Box<dyn FnOnce() + Send>>> {
    type Job = Box<dyn FnOnce() + Send>;
    static POOL: OnceLock<Option<std::sync::mpsc::Sender<Job>>> = OnceLock::new();
    POOL.get_or_init(|| {
        let (send, receive) = std::sync::mpsc::channel::<Job>();
        let receive = std::sync::Arc::new(Mutex::new(receive));
        let mut started = 0;
        for i in 0..5 {
            let receive = receive.clone();
            let spawned = std::thread::Builder::new()
                .name(format!("victory-fills-{i}"))
                .spawn(move || {
                    loop {
                        let job = receive.lock().unwrap_or_else(|e| e.into_inner()).recv();
                        match job {
                            // A share that panics is counted done
                            // (`ShareDone`); the thread takes the next.
                            Ok(job) => {
                                drop(std::panic::catch_unwind(std::panic::AssertUnwindSafe(job)))
                            }
                            Err(_) => return,
                        }
                    }
                });
            started += usize::from(spawned.is_ok());
        }
        (started > 0).then_some(send)
    })
    .as_ref()
}

/// Shares of a fill run left, and the wake-up when none is: one fewer when
/// dropped.
struct ShareDone(std::sync::Arc<(Mutex<usize>, std::sync::Condvar)>);

impl Drop for ShareDone {
    fn drop(&mut self) {
        let mut n = self.0.0.lock().unwrap_or_else(|e| e.into_inner());
        *n -= 1;
        if *n == 0 {
            self.0.1.notify_all();
        }
    }
}

/// Fills a thread takes at least: fewer are not worth one.
const FILLS_PER_JOB: usize = 128;

/// Whether `fills` can run in any order: their destinations rise without
/// overlapping (the upload memory a list hands out in turn), and no source
/// reaches into the destinations' span.
fn independent_fills(fills: &[Fill]) -> bool {
    let Some(first) = fills.first() else {
        return false;
    };
    let mut end = first.dst;
    for fill in fills {
        let Some(next) = fill.dst.checked_add(fill.len) else {
            return false;
        };
        if fill.dst < end {
            return false;
        }
        end = next;
    }
    fills.iter().all(|f| {
        f.src
            .checked_add(f.len)
            .is_some_and(|src_end| f.src >= end || src_end <= first.dst)
    })
}

/// Runs a run of fills: shared among up to `workers` threads, at least
/// [`FILLS_PER_JOB`] each, when they are independent, else in order. All
/// are done when it returns (before the next step reads or writes title
/// memory). Returns the threads used.
pub(super) fn run_fills(fills: &[Fill], workers: usize) -> usize {
    let jobs = workers.min(fills.len() / FILLS_PER_JOB);
    if jobs > 1
        && independent_fills(fills)
        && let Some(pool) = fill_pool()
    {
        let mut chunks = fills.chunks(fills.len().div_ceil(jobs));
        let mine = chunks.next().unwrap_or_default();
        let left = std::sync::Arc::new((Mutex::new(0usize), std::sync::Condvar::new()));
        for chunk in chunks {
            *left.0.lock().unwrap_or_else(|e| e.into_inner()) += 1;
            let (chunk, left) = (chunk.to_vec(), left.clone());
            let job: Box<dyn FnOnce() + Send> = Box::new(move || {
                // Counted done when dropped: after the share, or while a
                // panic in it unwinds (the submitting thread waits on it).
                let _done = ShareDone(left);
                chunk.into_iter().for_each(Fill::run);
            });
            if let Err(std::sync::mpsc::SendError(job)) = pool.send(job) {
                // No pool thread left: the share is done here.
                job();
            }
        }
        mine.iter().copied().for_each(Fill::run);
        let mut n = left.0.lock().unwrap_or_else(|e| e.into_inner());
        while *n > 0 {
            n = left.1.wait(n).unwrap_or_else(|e| e.into_inner());
        }
        FILL_STATS[0].fetch_add(1, Ordering::Relaxed);
        FILL_STATS[1].fetch_add(fills.len() as u64, Ordering::Relaxed);
        FILL_STATS[2].fetch_add(jobs as u64, Ordering::Relaxed);
        jobs
    } else {
        fills.iter().copied().for_each(Fill::run);
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fills of `bytes` each from `source` into consecutive pieces of `dst`.
    fn fills(source: &[u8], dst: &mut [u8], bytes: usize) -> Vec<Fill> {
        dst.chunks_exact_mut(bytes)
            .enumerate()
            .map(|(i, out)| Fill {
                dst: out.as_mut_ptr() as usize,
                src: source[i * bytes..].as_ptr() as usize,
                len: bytes,
            })
            .collect()
    }

    /// 512 fills of 448 bytes shared among four threads copy what one does,
    /// and the source as it is when they run.
    #[test]
    fn parallel_fills_copy_what_serial_ones_do() {
        let mut input: Vec<u8> = (0..512 * 448).map(|i| (i % 251) as u8).collect();
        let mut serial = vec![0; input.len()];
        let mut parallel = vec![0; input.len()];
        let serial_fills = fills(&input, &mut serial, 448);
        let parallel_fills = fills(&input, &mut parallel, 448);
        for turn in [0x5a, 0x37] {
            for b in &mut input {
                *b ^= turn;
            }
            assert_eq!(run_fills(&serial_fills, 1), 1);
            assert_eq!(run_fills(&parallel_fills, 4), 4);
            assert_eq!(parallel, input);
            assert_eq!(parallel, serial);
        }
    }

    /// A share that panics still counts as done: the thread waiting for
    /// the run's shares is woken.
    #[test]
    fn a_panicking_share_counts_as_done() {
        let left = std::sync::Arc::new((Mutex::new(1usize), std::sync::Condvar::new()));
        let done = ShareDone(left.clone());
        let share = std::thread::spawn(move || {
            let _done = done;
            panic!("a share that fails");
        });
        assert!(share.join().is_err());
        let n = left.0.lock().unwrap();
        let (n, waited) = left
            .1
            .wait_timeout_while(n, std::time::Duration::from_secs(5), |n| *n > 0)
            .unwrap();
        assert_eq!((*n, waited.timed_out()), (0, false));
    }

    /// A run of fills ends at the first other step: a title copy between
    /// fills splits them.
    #[test]
    fn a_title_copy_ends_a_run_of_fills() {
        let steps = [
            Step::Fill(0x1000, 0x9000, 16),
            Step::Fill(0x1010, 0x9010, 16),
            Step::Copy(0x9000, 0x5000, 16),
            Step::Fill(0x1020, 0x9000, 16),
        ];
        assert_eq!(fill_run(&steps).len(), 2);
        assert!(fill_run(&steps[2..]).is_empty());
        assert_eq!(fill_run(&steps[3..]).len(), 1);
    }

    /// Time of 28,279 fills of 448 bytes, as DexNom measured them (in runs
    /// of 512), on one thread and on four. Run with `--ignored --nocapture`.
    #[test]
    #[ignore]
    fn fill_runs_bench() {
        const FILLS: usize = 28_279;
        let source: Vec<u8> = (0..FILLS * 448).map(|i| i as u8).collect();
        let mut out = vec![0u8; source.len()];
        let all = fills(&source, &mut out, 448);
        let mut times = [Vec::new(), Vec::new()];
        for round in 0..20 {
            let workers = [1, 4][round % 2];
            let started = std::time::Instant::now();
            for run in all.chunks(512) {
                run_fills(run, workers);
            }
            times[round % 2].push(started.elapsed().as_secs_f64() * 1e3);
        }
        for (workers, mut t) in [1, 4].into_iter().zip(times) {
            t.sort_by(f64::total_cmp);
            eprintln!(
                "{workers} thread(s): median {:.2} ms, best {:.2} ms",
                t[t.len() / 2],
                t[0]
            );
        }
        assert_eq!(out, source);
    }

    /// Fills whose destinations overlap, or one reading another's
    /// destination, run in order: the last write wins, the read sees the
    /// earlier write.
    #[test]
    fn dependent_fills_run_in_order() {
        let source: Vec<u8> = (0..=255).collect();
        let mut output = [0; 256];
        let mut overlapping = fills(&source, &mut output, 1);
        overlapping[255].dst = overlapping[0].dst;
        assert_eq!(run_fills(&overlapping, 2), 1);
        assert_eq!(output[0], 255);
        let mut aliased = fills(&source, &mut output, 1);
        aliased[0].src = source[167..].as_ptr() as usize;
        aliased[1].src = aliased[0].dst;
        assert_eq!(run_fills(&aliased, 2), 1);
        assert_eq!(output[1], 167);
    }
}
