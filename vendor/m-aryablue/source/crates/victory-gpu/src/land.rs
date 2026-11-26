//! the relay's landing on several threads (VICTORY_WIDE_LAND=1, off by default)

use crate::shadow::Shadow;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};

// whether landings go wide
pub(crate) fn on() -> bool {
    // live: read once per landing, which waits for its pieces; the pool idles when off
    static S: crate::switch::Switch = crate::switch::Switch::new("WIDE_LAND");
    S.on()
}

// below this many bytes a landing stays on its own thread
const SMALL: usize = 256 << 10;

type Job = Box<dyn FnOnce() + Send + 'static>;

struct Pool {
    jobs: Mutex<std::collections::VecDeque<Job>>,
    more: Condvar,
    threads: usize,
}

fn pool() -> &'static Pool {
    static POOL: OnceLock<Pool> = OnceLock::new();
    static STARTED: std::sync::Once = std::sync::Once::new();
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get() / 2).clamp(2, 8) - 1;
    let pool = POOL.get_or_init(|| Pool {
        jobs: Mutex::new(std::collections::VecDeque::new()),
        more: Condvar::new(),
        threads,
    });
    STARTED.call_once(|| {
        for i in 0..pool.threads {
            let _ = std::thread::Builder::new().name(format!("victory-land-{i}")).spawn(|| {
                crate::placement::helper();
                let pool = POOL.get().expect("made above");
                loop {
                    let job = {
                        let mut jobs = pool.jobs.lock().unwrap_or_else(|e| e.into_inner());
                        loop {
                            if let Some(job) = jobs.pop_front() {
                                break job;
                            }
                            jobs = pool.more.wait(jobs).unwrap_or_else(|e| e.into_inner());
                        }
                    };
                    job();
                }
            });
        }
    });
    pool
}

// runs jobs on the pool and here; returns when all have run
fn run<'a>(jobs: Vec<Box<dyn FnOnce() + Send + 'a>>) {
    if jobs.len() < 2 {
        jobs.into_iter().for_each(|job| job());
        return;
    }
    let pool = pool();
    let left = Arc::new((Mutex::new(jobs.len()), Condvar::new()));
    let panicked = Arc::new(Mutex::new(None));
    {
        let mut queue = pool.jobs.lock().unwrap_or_else(|e| e.into_inner());
        for job in jobs {
            // SAFETY: waited for below, so what they borrow outlives them
            let job: Job = unsafe { std::mem::transmute::<Box<dyn FnOnce() + Send + 'a>, Job>(job) };
            let (left, panicked) = (left.clone(), panicked.clone());
            queue.push_back(Box::new(move || {
                if let Err(e) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job)) {
                    *panicked.lock().unwrap_or_else(|e| e.into_inner()) = Some(e);
                }
                let mut n = left.0.lock().unwrap_or_else(|e| e.into_inner());
                *n -= 1;
                if *n == 0 {
                    left.1.notify_all();
                }
            }));
        }
    }
    pool.more.notify_all();
    // only landings use this queue: helping with it takes no lock of theirs
    loop {
        let job = pool.jobs.lock().unwrap_or_else(|e| e.into_inner()).pop_front();
        match job {
            Some(job) => job(),
            None => break,
        }
    }
    let mut n = left.0.lock().unwrap_or_else(|e| e.into_inner());
    while *n > 0 {
        n = left.1.wait(n).unwrap_or_else(|e| e.into_inner());
    }
    drop(n);
    if let Some(e) = panicked.lock().unwrap_or_else(|e| e.into_inner()).take() {
        std::panic::resume_unwind(e);
    }
}

// pieces sharing a title byte (through others too) as groups, each in the order given
pub(crate) fn groups(spans: &[(usize, usize)]) -> Vec<Vec<usize>> {
    // empty ones share nothing: each alone
    let (mut order, empty): (Vec<usize>, Vec<usize>) = (0..spans.len()).partition(|&i| spans[i].0 < spans[i].1);
    order.sort_by_key(|&i| spans[i]);
    let mut out: Vec<Vec<usize>> = empty.into_iter().map(|i| vec![i]).collect();
    let mut open: Option<(usize, usize)> = None;
    for i in order {
        let (from, to) = spans[i];
        match open.as_mut() {
            Some((group, end)) if from < *end => {
                out[*group].push(i);
                *end = (*end).max(to);
            }
            _ => {
                out.push(vec![i]);
                open = Some((out.len() - 1, to));
            }
        }
    }
    for group in &mut out {
        group.sort_unstable();
    }
    out.sort_by_key(|g| g[0]);
    out
}

// groups as about `jobs` runs of work of like size, groups kept whole
pub(crate) fn pack(groups: Vec<Vec<usize>>, spans: &[(usize, usize)], jobs: usize) -> Vec<Vec<Vec<usize>>> {
    let size = |g: &Vec<usize>| g.iter().map(|&i| spans[i].1.saturating_sub(spans[i].0)).sum::<usize>();
    let total: usize = groups.iter().map(size).sum();
    let each = total.div_ceil(jobs.max(1)).max(1);
    let mut out: Vec<Vec<Vec<usize>>> = Vec::new();
    let mut bytes = 0;
    for group in groups {
        let n = size(&group);
        match out.last_mut() {
            Some(job) if bytes < each => job.push(group),
            _ => {
                out.push(vec![group]);
                bytes = 0;
            }
        }
        bytes += n;
    }
    out
}

// one landing at a time holds copies' locks
static ONE: Mutex<()> = Mutex::new(());

// merge(i, snapshot or 0, scratch, scratch) for every piece with all copies' locks held here; bytes each wrote
fn land(
    copies: &[&Shadow],
    spans: &[(usize, usize)],
    merge: &(dyn Fn(usize, usize, &mut Vec<u8>, &mut Vec<u8>) -> usize + Sync),
) -> Vec<usize> {
    let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
    let mut held: Vec<&Shadow> = copies.to_vec();
    held.sort_by_key(|s| *s as *const Shadow as usize);
    held.dedup_by_key(|s| *s as *const Shadow as usize);
    // in address order, as no other thread holds two of them
    let mut guards: Vec<_> = held.iter().map(|s| s.side_lock()).collect();
    let snapshots: Vec<(usize, usize)> = held
        .iter()
        .zip(guards.iter_mut())
        .map(|(s, g)| (*s as *const Shadow as usize, g.as_mut().map_or(0, |v| v.as_mut_ptr() as usize)))
        .collect();
    let snapshot = |i: usize| {
        let key = copies[i] as *const Shadow as usize;
        snapshots.binary_search_by_key(&key, |s| s.0).map_or(0, |at| snapshots[at].1)
    };
    let written: Vec<AtomicUsize> = (0..copies.len()).map(|_| AtomicUsize::new(0)).collect();
    let bytes: usize = spans.iter().map(|s| s.1.saturating_sub(s.0)).sum();
    if bytes < SMALL || copies.len() < 4 {
        let (mut now, mut title) = (Vec::new(), Vec::new());
        for i in 0..copies.len() {
            written[i].store(merge(i, snapshot(i), &mut now, &mut title), Ordering::Relaxed);
        }
    } else {
        let parts = pack(groups(spans), spans, (pool().threads + 1) * 2);
        let jobs: Vec<Box<dyn FnOnce() + Send + '_>> = parts
            .into_iter()
            .map(|part| {
                let (written, snapshot) = (&written, &snapshot);
                Box::new(move || {
                    let (mut now, mut title) = (Vec::new(), Vec::new());
                    for i in part.into_iter().flatten() {
                        written[i].store(merge(i, snapshot(i), &mut now, &mut title), Ordering::Relaxed);
                    }
                }) as Box<dyn FnOnce() + Send + '_>
            })
            .collect();
        run(jobs);
    }
    drop(guards);
    written.into_iter().map(AtomicUsize::into_inner).collect()
}

// card_merge_relay for every fetched piece (copy, lo, hi, at in data); bytes written
pub(crate) fn card(pieces: &[(Arc<Shadow>, usize, usize, u64)], data: usize, after: u64) -> usize {
    let copies: Vec<&Shadow> = pieces.iter().map(|p| &*p.0).collect();
    let spans: Vec<(usize, usize)> = pieces.iter().map(|p| (p.0.title_start() + p.1, p.0.title_start() + p.2)).collect();
    let merge = |i: usize, _: usize, _: &mut Vec<u8>, title: &mut Vec<u8>| {
        let (shadow, lo, hi, at) = &pieces[i];
        // SAFETY: the fetched bytes, whose copy has run
        let gpu = unsafe { std::slice::from_raw_parts((data + *at as usize) as *const u8, hi - lo) };
        let merged = shadow.card_merge_relay_held(*lo, gpu, after, title);
        shadow.walls_merged(gpu.len(), merged);
        merged
    };
    land(&copies, &spans, &merge).into_iter().sum()
}

// relay_merge for every piece (copy, at, n); bytes each wrote
pub(crate) fn pc(pieces: &[(&Arc<Shadow>, usize, usize)]) -> Vec<usize> {
    let copies: Vec<&Shadow> = pieces.iter().map(|p| &**p.0).collect();
    let spans: Vec<(usize, usize)> = pieces.iter().map(|p| (p.0.title_start() + p.1, p.0.title_start() + p.1 + p.2)).collect();
    let merge = |i: usize, snapshot: usize, now: &mut Vec<u8>, title: &mut Vec<u8>| {
        let (shadow, at, n) = pieces[i];
        if snapshot == 0 {
            return 0;
        }
        // SAFETY: this copy's snapshot, its lock held by land
        unsafe { shadow.relay_merge_held(at, n, snapshot as *mut u8, now, title) }
    };
    land(&copies, &spans, &merge)
}

#[cfg(test)]
mod tests {
    use super::{groups, pack, run};
    use crate::shadow::Shadow;
    use crate::shadow::card_merge_tests::test_copy;
    use std::sync::{Arc, Mutex};

    const LEN: usize = 1 << 20;
    const B_AT: usize = 300_000;
    const B_LEN: usize = 400_000;

    fn content() -> Vec<u8> {
        (0..LEN).map(|i| (i * 13 + 5) as u8).collect()
    }

    // what shaders left: sparse writes over all of a, denser ones over b (which shares title bytes with a)
    fn shader_bytes() -> (Vec<u8>, Vec<u8>) {
        let c = content();
        let mut a = c.clone();
        (0..LEN).step_by(97).for_each(|i| a[i] ^= 0x3c);
        let mut b = c[B_AT..B_AT + B_LEN].to_vec();
        (0..B_LEN).step_by(61).for_each(|i| b[i] ^= 0xa5);
        (a, b)
    }

    // a's pieces, b's in between, and a clipped piece of a again (overlapping one landed before)
    fn piece_list() -> Vec<(bool, usize, usize)> {
        let mut out: Vec<(bool, usize, usize)> = (0..8).map(|p| (true, p << 16, (p + 1) << 16)).collect();
        out.extend((0..B_LEN).step_by(1 << 16).map(|lo| (false, lo, (lo + (1 << 16)).min(B_LEN))));
        out.extend((8..16).map(|p| (true, p << 16, (p + 1) << 16)));
        out.push((true, (3 << 16) + 100, (4 << 16) - 100));
        out
    }

    // card copies: the wide landing leaves title memory and the cpu sides byte for byte as the serial one
    #[test]
    fn a_wide_card_landing_matches_the_serial_one() {
        let (card_a, card_b) = shader_bytes();
        let land = |wide: bool| -> Option<(Vec<u8>, Vec<u8>, Vec<u8>, usize)> {
            let c = content();
            let mut title = c.clone();
            // a title byte the title wrote since: kept
            title[123_457] ^= 0x11;
            let (mut side_a, mut side_b) = (c.clone(), c[B_AT..B_AT + B_LEN].to_vec());
            let base = title.as_ptr() as u64;
            // a title range sent up after the fetch: left to the title
            let a = test_copy(side_a.as_mut_ptr(), base, LEN as u64, Some(vec![(5000, 5100, 9)]), None)?;
            let b = test_copy(side_b.as_mut_ptr(), base + B_AT as u64, B_LEN as u64, Some(Vec::new()), None)?;
            let mut pieces: Vec<(Arc<Shadow>, usize, usize, u64)> = Vec::new();
            let mut data: Vec<u8> = Vec::new();
            for (in_a, lo, hi) in piece_list() {
                let (copy, card) = if in_a { (&a, &card_a) } else { (&b, &card_b) };
                pieces.push((copy.clone(), lo, hi, data.len() as u64));
                data.extend_from_slice(&card[lo..hi]);
            }
            let written = if wide {
                super::card(&pieces, data.as_ptr() as usize, 7)
            } else {
                let mut scratch = Vec::new();
                pieces
                    .iter()
                    .map(|(s, lo, hi, at)| s.card_merge_relay(*lo, &data[*at as usize..*at as usize + hi - lo], 7, &mut scratch))
                    .sum()
            };
            drop((pieces, a, b));
            Some((title, side_a, side_b, written))
        };
        let (Some(serial), Some(wide)) = (land(false), land(true)) else {
            return;
        };
        let c = content();
        assert!(serial.3 > 0 && serial.0 != c, "nothing landed");
        assert_eq!(serial.0[123_457], c[123_457] ^ 0x11, "the title's byte was lost");
        assert_eq!(serial.0[5044], c[5044], "a byte sent up after the fetch was taken");
        assert!(serial.0 == wide.0, "title memory differs");
        assert!(serial.1 == wide.1 && serial.2 == wide.2, "the cpu sides differ");
        assert_eq!(serial.3, wide.3);
    }

    // pc copies: the same with snapshots
    #[test]
    fn a_wide_pc_landing_matches_the_serial_one() {
        let (card_a, card_b) = shader_bytes();
        let land = |wide: bool| -> Option<(Vec<u8>, Vec<u8>, Vec<u8>, usize)> {
            let c = content();
            let mut title = c.clone();
            title[700_001] ^= 0x11;
            let (mut copy_a, mut copy_b) = (card_a.clone(), card_b.clone());
            let base = title.as_ptr() as u64;
            let a = test_copy(copy_a.as_mut_ptr(), base, LEN as u64, None, Some(c.clone()))?;
            let b = test_copy(copy_b.as_mut_ptr(), base + B_AT as u64, B_LEN as u64, None, Some(c[B_AT..B_AT + B_LEN].to_vec()))?;
            let pieces: Vec<(&Arc<Shadow>, usize, usize)> =
                piece_list().into_iter().map(|(in_a, lo, hi)| (if in_a { &a } else { &b }, lo, hi - lo)).collect();
            let written: usize = if wide {
                super::pc(&pieces).into_iter().sum()
            } else {
                let (mut now, mut scratch) = (Vec::new(), Vec::new());
                pieces.iter().map(|(s, at, n)| s.relay_merge(*at, *n, &mut now, &mut scratch)).sum()
            };
            let snaps = [&a, &b].map(|s| s.side_lock().clone().unwrap_or_default());
            drop(pieces);
            drop((a, b));
            let [sa, sb] = snaps;
            Some((title, sa, sb, written))
        };
        let (Some(serial), Some(wide)) = (land(false), land(true)) else {
            return;
        };
        let c = content();
        assert!(serial.3 > 0 && serial.0 != c, "nothing landed");
        assert_eq!(serial.0[700_001], c[700_001] ^ 0x11, "the title's byte was lost");
        assert!(serial.0 == wide.0, "title memory differs");
        assert!(serial.1 == wide.1 && serial.2 == wide.2, "the snapshots differ");
        assert_eq!(serial.3, wide.3);
    }

    // overlapping pieces share a group in their own order; apart ones do not
    #[test]
    fn overlapping_pieces_stay_together_in_order() {
        let spans = [(100, 200), (0, 50), (150, 300), (50, 100), (299, 310), (400, 500), (40, 60), (5, 5)];
        let g = groups(&spans);
        assert_eq!(g, vec![vec![0, 2, 4], vec![1, 3, 6], vec![5], vec![7]]);
        let all: Vec<usize> = {
            let mut a: Vec<usize> = g.iter().flatten().copied().collect();
            a.sort_unstable();
            a
        };
        assert_eq!(all, (0..spans.len()).collect::<Vec<_>>());
        // no two groups share a byte
        for (x, a) in g.iter().enumerate() {
            for b in &g[x + 1..] {
                for &i in a {
                    for &j in b {
                        let (s, t) = (spans[i], spans[j]);
                        assert!(s.0 >= s.1 || t.0 >= t.1 || s.1 <= t.0 || t.1 <= s.0, "{s:?} {t:?}");
                    }
                }
            }
        }
    }

    // packing keeps every group whole and each once
    #[test]
    fn packing_keeps_groups_whole() {
        let spans: Vec<(usize, usize)> = (0..40).map(|i| (i * 100, i * 100 + 10 + i)).collect();
        let g = groups(&spans);
        let parts = pack(g.clone(), &spans, 6);
        assert!(parts.len() > 1 && parts.len() <= 7);
        let flat: Vec<Vec<usize>> = parts.into_iter().flatten().collect();
        assert_eq!(flat, g);
    }

    // the pool runs every job and returns after the last
    #[test]
    fn run_waits_for_every_job() {
        let seen = Mutex::new(Vec::new());
        let threads = Mutex::new(std::collections::HashSet::new());
        let jobs: Vec<Box<dyn FnOnce() + Send + '_>> = (0..64)
            .map(|i| {
                let (seen, threads) = (&seen, &threads);
                Box::new(move || {
                    std::thread::sleep(std::time::Duration::from_millis(2));
                    threads.lock().unwrap().insert(std::thread::current().id());
                    seen.lock().unwrap().push(i);
                }) as Box<dyn FnOnce() + Send + '_>
            })
            .collect();
        run(jobs);
        let mut seen = seen.into_inner().unwrap();
        seen.sort_unstable();
        assert_eq!(seen, (0..64).collect::<Vec<_>>());
        assert!(threads.into_inner().unwrap().len() > 1, "ran on one thread");
    }

    // offline timing, serial against wide: cargo test --release land::tests::timing -- --ignored --nocapture
    #[test]
    #[ignore]
    fn timing() {
        const BIG: usize = 64 << 20;
        let c: Vec<u8> = (0..BIG).map(|i| (i * 13 + 5) as u8).collect();
        let mut card = c.clone();
        (0..BIG).step_by(7).for_each(|i| card[i] ^= 0x5a);
        for wide in [false, true, false, true] {
            let mut title = c.clone();
            let mut side = c.clone();
            let Some(a) = test_copy(side.as_mut_ptr(), title.as_mut_ptr() as u64, BIG as u64, Some(Vec::new()), None) else {
                return;
            };
            let pieces: Vec<(Arc<Shadow>, usize, usize, u64)> = (0..BIG >> 16).map(|p| (a.clone(), p << 16, (p + 1) << 16, (p << 16) as u64)).collect();
            let at = std::time::Instant::now();
            let written = if wide {
                super::card(&pieces, card.as_ptr() as usize, 0)
            } else {
                let mut scratch = Vec::new();
                pieces.iter().map(|(s, lo, hi, at)| s.card_merge_relay(*lo, &card[*at as usize..*at as usize + hi - lo], 0, &mut scratch)).sum()
            };
            println!("wide {wide}: {written} bytes in {:.1} ms", at.elapsed().as_secs_f64() * 1000.0);
            assert!(title == card);
            drop(pieces);
            drop(a);
            drop((title, side));
        }
    }
}
