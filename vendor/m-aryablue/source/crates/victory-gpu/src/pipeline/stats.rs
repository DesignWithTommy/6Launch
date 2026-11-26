//! Where the pipeline workers' time goes: per make, wall and processor
//! time, and the wall time of its parts (translating shaders, loading from
//! the library, the driver's compile, storing). Logged every 10 s while
//! there is work, with the queue and the title's waiting.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// The parts of a make, in microseconds of wall time.
#[derive(Clone, Copy)]
pub(crate) enum Part {
    Translate = 0,
    Load = 1,
    Create = 2,
    Store = 3,
    Save = 4,
}

const PARTS: usize = 5;
static PART_US: [AtomicU64; PARTS] = [const { AtomicU64::new(0) }; PARTS];
/// Makes, their wall and processor microseconds, and the title's waiting.
static MAKES: AtomicU64 = AtomicU64::new(0);
static WALL_US: AtomicU64 = AtomicU64::new(0);
static CPU_US: AtomicU64 = AtomicU64::new(0);
static WAITED_US: AtomicU64 = AtomicU64::new(0);
static WAITS: AtomicU64 = AtomicU64::new(0);

/// Times `f` as `part` of the make on this thread.
pub(crate) fn part<T>(part: Part, f: impl FnOnce() -> T) -> T {
    let began = Instant::now();
    let out = f();
    PART_US[part as usize].fetch_add(began.elapsed().as_micros() as u64, Ordering::Relaxed);
    out
}

/// A title thread waited `waited` for a pipeline.
pub(crate) fn waited(waited: Duration) {
    WAITS.fetch_add(1, Ordering::Relaxed);
    WAITED_US.fetch_add(waited.as_micros() as u64, Ordering::Relaxed);
}

/// A driver compile of a second or more, named by its shaders.
pub(crate) fn slow_compile(took: Duration, label: impl FnOnce() -> String) {
    if took >= Duration::from_secs(1) {
        let label = label();
        tracing::info!(
            ms = took.as_millis() as u64,
            "slow driver compile: {}",
            label.chars().take(300).collect::<String>()
        );
    }
}

/// This thread's processor time so far, in microseconds.
fn cpu_us() -> u64 {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::Threading::{GetCurrentThread, GetThreadTimes};
    let zero = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
    // SAFETY: our own thread and four valid out pointers.
    unsafe {
        GetThreadTimes(
            GetCurrentThread(),
            &mut created,
            &mut exited,
            &mut kernel,
            &mut user,
        )
    };
    let us = |t: FILETIME| (u64::from(t.dwHighDateTime) << 32 | u64::from(t.dwLowDateTime)) / 10;
    us(user) + us(kernel)
}

/// Runs one worker make and counts it.
pub(crate) fn make(f: impl FnOnce()) {
    let (began, cpu) = (Instant::now(), cpu_us());
    f();
    MAKES.fetch_add(1, Ordering::Relaxed);
    WALL_US.fetch_add(began.elapsed().as_micros() as u64, Ordering::Relaxed);
    CPU_US.fetch_add(cpu_us().saturating_sub(cpu), Ordering::Relaxed);
}

/// One line every 10 s (called by the workers after each make).
pub(crate) fn maybe_log(requested: usize, early: usize, waiting: u64) {
    static LAST: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);
    {
        let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
        match *last {
            Some(t) if t.elapsed() < Duration::from_secs(10) => return,
            None => {
                *last = Some(Instant::now());
                return;
            }
            _ => *last = Some(Instant::now()),
        }
    }
    let ms = |a: &AtomicU64| a.swap(0, Ordering::Relaxed) as f64 / 1e3;
    let makes = MAKES.swap(0, Ordering::Relaxed);
    let (wall, cpu) = (ms(&WALL_US), ms(&CPU_US));
    let [translate, load, create, store, save] = [0, 1, 2, 3, 4].map(|i| ms(&PART_US[i]));
    let (waits, waited) = (WAITS.swap(0, Ordering::Relaxed), ms(&WAITED_US));
    tracing::info!(
        "pipeline workers, last 10 s: {makes} made, {wall:.0} ms wall, {cpu:.0} ms processor \
         ({:.0}%); parts (wall): translating {translate:.0} ms, library loads {load:.0} ms, \
         driver compiles {create:.0} ms, stores {store:.0} ms, library saves {save:.0} ms; queue: {requested} requested, \
         {early} early, {waiting} not made; the title waited {waited:.0} ms in {waits} waits",
        if wall > 0.0 { cpu * 100.0 / wall } else { 0.0 },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_busy_make_counts_processor_time_and_a_sleep_does_not() {
        let before = CPU_US.load(Ordering::Relaxed);
        make(|| std::thread::sleep(Duration::from_millis(50)));
        let slept = CPU_US.load(Ordering::Relaxed) - before;
        let before = CPU_US.load(Ordering::Relaxed);
        make(|| {
            let t = Instant::now();
            let mut x = 0u64;
            while t.elapsed() < Duration::from_millis(50) {
                x = std::hint::black_box(x.wrapping_add(1));
            }
        });
        let spun = CPU_US.load(Ordering::Relaxed) - before;
        assert!(spun > slept + 20_000, "spun {spun} us, slept {slept} us");
    }
}
