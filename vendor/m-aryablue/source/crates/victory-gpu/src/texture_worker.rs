//! Textures picked by index made ready off the recording thread (`VICTORY_TEXTURE_WORKER=1`):
//! a worker makes the staging buffer and detiles into it, and the texture pass only records
//! the copy once that is done.

use std::collections::HashSet;
use std::sync::Mutex;

use windows::Win32::Graphics::Direct3D12::ID3D12Resource;

use crate::object::This;

/// Whether the texture pass hands its textures to the worker.
pub(crate) fn on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_TEXTURE_WORKER").is_some_and(|v| v == "1"))
}

/// Most textures handed to the worker and not yet done.
const MOST: usize = 1024;

/// Since the last frame-rate line: textures handed over, made ready, refused for a full queue,
/// and microseconds the worker spent.
pub(crate) static STATS: [std::sync::atomic::AtomicU64; 4] = [const { std::sync::atomic::AtomicU64::new(0) }; 4];

/// What is with the worker: each key once, at most `most` of them.
pub(crate) struct Pending {
    keys: HashSet<usize>,
    most: usize,
}

impl Pending {
    pub(crate) fn new(most: usize) -> Self {
        Pending { keys: HashSet::new(), most }
    }

    /// Whether `key` is to be handed over now (not already with the worker, and room for it).
    pub(crate) fn take(&mut self, key: usize) -> bool {
        if self.keys.contains(&key) || self.keys.len() >= self.most {
            return false;
        }
        self.keys.insert(key)
    }

    pub(crate) fn done(&mut self, key: usize) {
        self.keys.remove(&key);
    }
}

/// Hands waiting for the worker, and whether it takes the newest first.
struct Jobs<T> {
    queue: std::collections::VecDeque<(usize, T)>,
    newest: bool,
}

impl<T> Jobs<T> {
    fn next(&mut self) -> Option<(usize, T)> {
        if self.newest { self.queue.pop_back() } else { self.queue.pop_front() }
    }
}

/// A worker thread that runs `prepare` on what it is handed, one at a time.
pub(crate) struct Worker<T: Send + 'static> {
    pending: std::sync::Arc<Mutex<Pending>>,
    jobs: std::sync::Arc<(Mutex<Jobs<T>>, std::sync::Condvar)>,
}

impl<T: Send + 'static> Worker<T> {
    /// `newest`: the last hand first (the pass hands the slots of the newest writes first each
    /// frame, so a backlog of old hands no longer holds up what this frame shows).
    pub(crate) fn new(most: usize, newest: bool, prepare: impl Fn(usize, T) + Send + 'static) -> Self {
        let pending = std::sync::Arc::new(Mutex::new(Pending::new(most)));
        let jobs = std::sync::Arc::new((
            Mutex::new(Jobs { queue: std::collections::VecDeque::new(), newest }),
            std::sync::Condvar::new(),
        ));
        let (left, mine) = (pending.clone(), jobs.clone());
        std::thread::Builder::new()
            .name("victory texture worker".into())
            .spawn(move || {
                crate::placement::helper();
                loop {
                    let (key, job) = {
                        let (lock, ready) = &*mine;
                        let mut jobs = lock.lock().unwrap_or_else(|e| e.into_inner());
                        loop {
                            if let Some(next) = jobs.next() {
                                break next;
                            }
                            jobs = ready.wait(jobs).unwrap_or_else(|e| e.into_inner());
                        }
                    };
                    let started = std::time::Instant::now();
                    prepare(key, job);
                    STATS[3].fetch_add(started.elapsed().as_micros() as u64, std::sync::atomic::Ordering::Relaxed);
                    left.lock().unwrap_or_else(|e| e.into_inner()).done(key);
                }
            })
            .expect("texture worker thread");
        Worker { pending, jobs }
    }

    /// Hands `key` over unless it is with the worker already or the queue is full; true if handed.
    pub(crate) fn hand(&self, key: usize, job: impl FnOnce() -> T) -> bool {
        use std::sync::atomic::Ordering::Relaxed;
        if !self.pending.lock().unwrap_or_else(|e| e.into_inner()).take(key) {
            STATS[2].fetch_add(1, Relaxed);
            return false;
        }
        STATS[0].fetch_add(1, Relaxed);
        let (lock, ready) = &*self.jobs;
        lock.lock().unwrap_or_else(|e| e.into_inner()).queue.push_back((key, job()));
        ready.notify_one();
        true
    }

    pub(crate) fn busy(&self, key: usize) -> bool {
        self.pending.lock().unwrap_or_else(|e| e.into_inner()).keys.contains(&key)
    }
}

/// VICTORY_TEXTURE_WORKER_NEWEST=1: the worker takes its newest hand first (off by default).
fn newest_first() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_TEXTURE_WORKER_NEWEST").is_some_and(|v| v == "1"))
}

/// A host texture for the worker, and its hand's token (`crate::texture::handed`).
struct Job(ID3D12Resource, u64);

// SAFETY: D3D12 resources are free-threaded; the worker only reads its description and maps
// a staging buffer for it.
unsafe impl Send for Job {}

fn worker() -> &'static Worker<Job> {
    static WORKER: std::sync::OnceLock<Worker<Job>> = std::sync::OnceLock::new();
    WORKER.get_or_init(|| {
        Worker::new(MOST, newest_first(), |resource, Job(texture, token)| {
            crate::texture::prestage(resource as This, &texture, token);
            STATS[1].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        })
    })
}

/// Whether the pass records `resource`'s upload now: its staging is ready, or it is not one for
/// the worker (levels on demand); else it is handed to the worker (once) and waits.
pub(crate) fn ready(resource: This, texture: &ID3D12Resource) -> bool {
    if crate::texture::prepared(resource) || crate::resource::host_tiled(resource).is_some() {
        return true;
    }
    let worker = worker();
    if !worker.busy(resource as usize) {
        worker.hand(resource as usize, || Job(texture.clone(), crate::texture::handed(resource)));
    }
    false
}

fn arrivals() -> &'static Worker<Job> {
    static WORKER: std::sync::OnceLock<Worker<Job>> = std::sync::OnceLock::new();
    WORKER.get_or_init(|| {
        Worker::new(MOST, true, |resource, Job(texture, token)| {
            crate::texture::restage(resource as This, &texture, token);
        })
    })
}

/// DirectStorage landed bytes in `resource`'s memory (`crate::texture_hold::landed`): it is read
/// again on a worker now; false if it is with it already or its queue is full.
pub(crate) fn arrived(resource: This, texture: ID3D12Resource) -> bool {
    let worker = arrivals();
    !worker.busy(resource as usize) && worker.hand(resource as usize, || Job(texture, crate::texture::handed(resource)))
}

#[cfg(test)]
mod tests {
    use super::{Jobs, Pending, Worker};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, mpsc};
    use std::time::Duration;

    #[test]
    fn a_texture_is_handed_over_once_until_done_and_not_past_the_most() {
        let mut p = Pending::new(2);
        assert!(p.take(1));
        assert!(!p.take(1), "handed over twice");
        assert!(p.take(2));
        assert!(!p.take(3), "past the most");
        p.done(1);
        assert!(p.take(3));
        assert!(!p.take(1), "full again");
    }

    #[test]
    fn the_worker_prepares_off_the_calling_thread_and_frees_the_key() {
        let caller = std::thread::current().id();
        let (tell, told) = mpsc::channel();
        let runs = Arc::new(AtomicUsize::new(0));
        let counted = runs.clone();
        let w = Worker::new(8, false, move |key, value: u32| {
            counted.fetch_add(1, Ordering::SeqCst);
            let _ = tell.send((key, value, std::thread::current().id()));
        });
        assert!(w.hand(7, || 70));
        let (key, value, thread) = told.recv_timeout(Duration::from_secs(5)).expect("prepared");
        assert_eq!((key, value), (7, 70));
        assert_ne!(thread, caller, "prepared on the calling thread");
        for _ in 0..500 {
            if !w.busy(7) {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(!w.busy(7), "the key stayed taken");
        assert!(w.hand(7, || 71), "not handed again after it was done");
        told.recv_timeout(Duration::from_secs(5)).expect("prepared again");
        assert_eq!(runs.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn the_newest_hand_is_taken_first_when_asked() {
        let mut jobs = Jobs { queue: (1..=3).map(|k| (k, ())).collect(), newest: true };
        assert_eq!(jobs.next().map(|j| j.0), Some(3));
        assert_eq!(jobs.next().map(|j| j.0), Some(2));
        let mut jobs = Jobs { queue: (1..=3).map(|k| (k, ())).collect(), newest: false };
        assert_eq!(jobs.next().map(|j| j.0), Some(1));
    }
}
