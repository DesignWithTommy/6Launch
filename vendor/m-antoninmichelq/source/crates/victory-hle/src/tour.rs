//! Debugging aid, the map tour: `VICTORY_TOUR=<points>` moves the player to
//! one place of the map after another, waits at each until the streaming
//! has settled (draws per frame steady, no host pipeline left to make),
//! saves a picture (with `VICTORY_CAPTURE`) and writes a line of
//! `tour-results.csv`: the textures and shaders of the whole map are met and
//! their host pipelines made and cached, and each place says what is wrong
//! there (few draws, textures left waiting, memory, a crash).
//!
//! `<points>`: a file of `x y z [name]` lines (`#` comments), or
//! `grid:<step m>:<rings>`, a square spiral around where the player
//! stands, `VICTORY_TOUR_HEIGHT` (40) meters above them.
//!
//! The player's place is not known to us: it is found in the title's memory
//! as Cheat Engine users do, with the title's own walk. Every 4x4 matrix
//! (rotation rows of length 1 at right angles, then a position) in
//! writable memory is a candidate; the player is walked forward (W), held,
//! then walked back (S); the matrices whose position followed are the
//! player's (and the camera's, which follows them). At each place their
//! positions are written over and over (every 15 ms), so the player hangs
//! there while the place streams in; nothing else of the title is changed.
//!
//! Other switches: `VICTORY_TOUR_WAIT` seconds after the world's first
//! frames before looking for the player (120: time for the menus, the
//! save's loading and a cutscene); `VICTORY_TOUR_MIN`/`VICTORY_TOUR_MAX`
//! seconds at a place (45/240); `VICTORY_TOUR_DIR` the folder for
//! `tour-state.txt` and `tour-results.csv` (`VICTORY_CAPTURE` or the working
//! folder). `tour-state.txt` remembers the places done: a run started again
//! goes on with the next one, and a place where the last run ended without
//! finishing it (a crash) is reported and passed over.

use std::io::Write as _;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use victory_gpu::tour_hooks as frames;
use windows_sys::Win32::System::Diagnostics::Debug::{ReadProcessMemory, WriteProcessMemory};
use windows_sys::Win32::System::Memory::{
    MEM_COMMIT, MEMORY_BASIC_INFORMATION, PAGE_GUARD, PAGE_NOCACHE, PAGE_READWRITE,
    PAGE_WRITECOMBINE, VirtualQuery,
};
use windows_sys::Win32::System::ProcessStatus::{
    K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
};
use windows_sys::Win32::System::Threading::GetCurrentProcess;

use crate::modules::gameinput::hold_keys;

/// A place to visit.
#[derive(Clone, Debug)]
struct Point {
    name: String,
    at: [f32; 3],
}

/// Where the points come from.
enum Points {
    Listed(Vec<Point>),
    /// Step in meters, rings around the start.
    Grid(f32, u32),
}

/// Starts the tour on a thread of its own, if `VICTORY_TOUR` is set.
pub fn start() {
    let Ok(spec) = std::env::var("VICTORY_TOUR") else {
        return;
    };
    let points = match parse_points(&spec) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(error = %e, "tour: no points, no tour");
            return;
        }
    };
    let _ = std::thread::Builder::new()
        .name("map tour".into())
        .spawn(move || run(points));
}

fn parse_points(spec: &str) -> Result<Points, String> {
    if let Some(grid) = spec.strip_prefix("grid:") {
        let (step, rings) = grid.split_once(':').ok_or("grid:<step>:<rings>")?;
        let step: f32 = step.trim().parse().map_err(|_| "grid step")?;
        let rings: u32 = rings.trim().parse().map_err(|_| "grid rings")?;
        return Ok(Points::Grid(step, rings));
    }
    let text = std::fs::read_to_string(spec).map_err(|e| format!("{spec}: {e}"))?;
    let mut points = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let mut words = line.split_whitespace();
        let mut at = [0.0f32; 3];
        for v in &mut at {
            *v = words
                .next()
                .and_then(|w| w.parse().ok())
                .ok_or_else(|| format!("{spec} line {}: x y z [name]", n + 1))?;
        }
        let name = words.collect::<Vec<_>>().join(" ");
        let name = if name.is_empty() { format!("p{}", points.len()) } else { name };
        points.push(Point { name, at });
    }
    Ok(Points::Listed(points))
}

/// The square spiral: the center, then each ring's points.
fn grid(center: [f32; 3], step: f32, rings: u32, height: f32) -> Vec<Point> {
    let mut points = Vec::new();
    for r in 0..=rings as i32 {
        let mut ring = Vec::new();
        if r == 0 {
            ring.push((0, 0));
        } else {
            for i in -r..r {
                ring.push((i, -r));
            }
            for i in -r..r {
                ring.push((r, i));
            }
            for i in (-r + 1..=r).rev() {
                ring.push((i, r));
            }
            for i in (-r + 1..=r).rev() {
                ring.push((-r, i));
            }
        }
        for (i, j) in ring {
            points.push(Point {
                name: format!("grid {i} {j}"),
                at: [
                    center[0] + i as f32 * step,
                    center[1] + j as f32 * step,
                    center[2] + height,
                ],
            });
        }
    }
    points
}

fn env_secs(name: &str, default: f64) -> Duration {
    let secs = std::env::var(name)
        .ok()
        .and_then(|v| v.trim().parse::<f64>().ok())
        .unwrap_or(default);
    Duration::from_secs_f64(secs.max(0.0))
}

fn out_dir() -> PathBuf {
    std::env::var_os("VICTORY_TOUR_DIR")
        .or_else(|| std::env::var_os("VICTORY_CAPTURE"))
        .map_or_else(|| PathBuf::from("."), PathBuf::from)
}

/// Frames, draws and the like at one moment.
#[derive(Clone, Copy)]
struct Sample {
    when: Instant,
    frames: u64,
    drawn: u64,
}

impl Sample {
    fn now() -> Self {
        Self {
            when: Instant::now(),
            frames: frames::presented(),
            drawn: frames::drawn(),
        }
    }

    /// Draws per frame and frames per second since `before`.
    fn since(&self, before: &Sample) -> (f64, f64) {
        let frames = self.frames.saturating_sub(before.frames).max(1) as f64;
        let draws = self.drawn.saturating_sub(before.drawn) as f64 / frames;
        let secs = self.when.duration_since(before.when).as_secs_f64().max(1e-3);
        (draws, frames / secs)
    }
}

fn private_mb() -> u64 {
    let mut counters: PROCESS_MEMORY_COUNTERS_EX = unsafe { std::mem::zeroed() };
    counters.cb = size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
    // SAFETY: our own process, a counters block of the size named.
    let ok = unsafe {
        K32GetProcessMemoryInfo(
            GetCurrentProcess(),
            (&raw mut counters).cast::<PROCESS_MEMORY_COUNTERS>(),
            counters.cb,
        )
    };
    if ok == 0 { 0 } else { counters.PrivateUsage as u64 >> 20 }
}

fn run(points: Points) {
    let dir = out_dir();
    let _ = std::fs::create_dir_all(&dir);
    tracing::info!(dir = %dir.display(), "tour: waiting for the world");
    // The world: a few seconds of 500 draws a frame or more.
    let mut before = Sample::now();
    loop {
        std::thread::sleep(Duration::from_secs(5));
        let now = Sample::now();
        let (draws, _) = now.since(&before);
        before = now;
        if draws >= 500.0 {
            break;
        }
    }
    let wait = env_secs("VICTORY_TOUR_WAIT", 120.0);
    tracing::info!(seconds = wait.as_secs(), "tour: the world is drawn; looking for the player after");
    std::thread::sleep(wait);

    let mut player = None;
    for attempt in 1..=4 {
        match find_player() {
            Ok(found) => {
                player = Some(found);
                break;
            }
            Err(e) => {
                tracing::warn!(attempt, error = %e, "tour: player not found; trying again in 60 s");
                std::thread::sleep(Duration::from_secs(60));
            }
        }
    }
    let Some(player) = player else {
        tracing::warn!("tour: the player was not found; no tour");
        return;
    };
    let start = player.position();
    tracing::info!(?start, matrices = player.addresses.len(), "tour: the player");

    let height = std::env::var("VICTORY_TOUR_HEIGHT")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(40.0);
    let points = match points {
        Points::Listed(p) => p,
        Points::Grid(step, rings) => grid(start, step, rings, height),
    };

    // Places done, and the one a run ended in.
    let state_path = dir.join("tour-state.txt");
    let state = std::fs::read_to_string(&state_path).unwrap_or_default();
    let done: Vec<usize> = state
        .lines()
        .filter_map(|l| l.strip_prefix("done ")?.trim().parse().ok())
        .collect();
    let crashed: Vec<usize> = state
        .lines()
        .filter_map(|l| l.strip_prefix("start ")?.trim().parse().ok())
        .filter(|i| !done.contains(i))
        .collect();
    let results_path = dir.join("tour-results.csv");
    let new_file = !results_path.exists();
    let Ok(mut results) = std::fs::OpenOptions::new().create(true).append(true).open(&results_path) else {
        tracing::warn!(path = %results_path.display(), "tour: results file not opened; no tour");
        return;
    };
    if new_file {
        let _ = writeln!(
            results,
            "point,name,x,y,z,seconds,settled,draws_per_frame,fps,textures_waiting,pipelines_waiting,private_mb,frame"
        );
    }
    for &i in &crashed {
        if let Some(p) = points.get(i) {
            tracing::warn!(point = i, name = %p.name, "tour: the last run ended at this place; passed over");
            let _ = writeln!(
                results,
                "{i},{},{},{},{},,crashed,,,,,,",
                p.name, p.at[0], p.at[1], p.at[2]
            );
        }
    }
    let mut state = std::fs::OpenOptions::new().create(true).append(true).open(&state_path).ok();
    for &i in &crashed {
        if let Some(s) = state.as_mut() {
            let _ = writeln!(s, "done {i}");
        }
    }

    let min = env_secs("VICTORY_TOUR_MIN", 45.0);
    let max = env_secs("VICTORY_TOUR_MAX", 240.0);
    for (i, point) in points.iter().enumerate() {
        if done.contains(&i) || crashed.contains(&i) {
            continue;
        }
        tracing::info!(point = i, of = points.len(), name = %point.name, at = ?point.at, "tour: going to");
        if let Some(s) = state.as_mut() {
            let _ = writeln!(s, "start {i}");
            let _ = s.flush();
        }
        let hold = player.hold_at(point.at);
        let arrived = Instant::now();
        let mut windows: Vec<(f64, f64)> = Vec::new();
        let mut before = Sample::now();
        let settled = loop {
            std::thread::sleep(Duration::from_secs(5));
            let now = Sample::now();
            windows.push(now.since(&before));
            before = now;
            let stay = arrived.elapsed();
            let steady = windows.len() >= 3 && {
                let last = &windows[windows.len() - 3..];
                let (lo, hi) = last.iter().fold((f64::MAX, 0.0f64), |(lo, hi), w| (lo.min(w.0), hi.max(w.0)));
                hi > 0.0 && (hi - lo) / hi < 0.05
            };
            if stay >= min && steady && frames::pipelines_waiting() == 0 {
                break true;
            }
            if stay >= max {
                break false;
            }
        };
        let frame = frames::presented() + 2;
        frames::save_frame(frame);
        std::thread::sleep(Duration::from_secs(3));
        let (draws, fps) = windows.last().copied().unwrap_or_default();
        let line = format!(
            "{i},{},{},{},{},{},{},{:.0},{:.1},{},{},{},{}",
            point.name,
            point.at[0],
            point.at[1],
            point.at[2],
            arrived.elapsed().as_secs(),
            if settled { "yes" } else { "no" },
            draws,
            fps,
            frames::textures_waiting(),
            frames::pipelines_waiting(),
            private_mb(),
            frame
        );
        tracing::info!("tour: {line}");
        let _ = writeln!(results, "{line}");
        let _ = results.flush();
        // Three more views, turned a quarter each (right stick).
        for turn in 1..=3 {
            hold_keys(b"L");
            std::thread::sleep(Duration::from_millis(1500));
            hold_keys(&[]);
            std::thread::sleep(Duration::from_secs(20));
            let frame = frames::presented() + 2;
            frames::save_frame(frame);
            tracing::info!(point = i, turn, frame, "tour: view saved");
            std::thread::sleep(Duration::from_secs(2));
        }
        drop(hold);
        if let Some(s) = state.as_mut() {
            let _ = writeln!(s, "done {i}");
            let _ = s.flush();
        }
    }
    tracing::info!(results = %results_path.display(), "tour: finished");
}

/// The player's matrices: their addresses in the title's memory.
struct Player {
    addresses: Vec<usize>,
}

/// Writes a position over and over while held.
struct Hold {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for Hold {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Player {
    fn position(&self) -> [f32; 3] {
        self.addresses
            .iter()
            .find_map(|&a| read_matrix(a).map(|m| [m[12], m[13], m[14]]))
            .unwrap_or_default()
    }

    fn hold_at(&self, at: [f32; 3]) -> Hold {
        let stop = Arc::new(AtomicBool::new(false));
        let addresses = self.addresses.clone();
        let flag = stop.clone();
        let thread = std::thread::Builder::new()
            .name("map tour hold".into())
            .spawn(move || {
                let bytes: Vec<u8> = at.iter().flat_map(|v| v.to_le_bytes()).collect();
                while !flag.load(Ordering::Relaxed) {
                    for &a in &addresses {
                        write_bytes(a + 48, &bytes);
                    }
                    std::thread::sleep(Duration::from_millis(15));
                }
            })
            .ok();
        Hold { stop, thread }
    }
}

fn read_bytes(at: usize, out: &mut [u8]) -> bool {
    let mut read = 0;
    // SAFETY: our own process; a page that went away fails the call
    // instead of faulting.
    let ok = unsafe {
        ReadProcessMemory(GetCurrentProcess(), at as _, out.as_mut_ptr().cast(), out.len(), &mut read)
    };
    ok != 0 && read == out.len()
}

fn write_bytes(at: usize, bytes: &[u8]) -> bool {
    let mut written = 0;
    // SAFETY: as in `read_bytes`; only the twelve position bytes of a
    // matrix found to be the player's are written.
    let ok = unsafe {
        WriteProcessMemory(GetCurrentProcess(), at as _, bytes.as_ptr().cast(), bytes.len(), &mut written)
    };
    ok != 0 && written == bytes.len()
}

fn read_matrix(at: usize) -> Option<[f32; 16]> {
    let mut raw = [0u8; 64];
    if !read_bytes(at, &mut raw) {
        return None;
    }
    let m: [f32; 16] = std::array::from_fn(|i| f32::from_le_bytes(raw[i * 4..i * 4 + 4].try_into().unwrap()));
    is_placement(&m).then_some(m)
}

/// A rotation (three rows of length 1 at right angles, right-handed) and
/// a position inside the map.
fn is_placement(m: &[f32; 16]) -> bool {
    let row = |r: usize| [m[r * 4], m[r * 4 + 1], m[r * 4 + 2]];
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let (a, b, c) = (row(0), row(1), row(2));
    let unit = |v: [f32; 3]| (0.96..=1.04).contains(&dot(v, v));
    if !(unit(a) && unit(b) && unit(c)) {
        return false;
    }
    if dot(a, b).abs() > 0.03 || dot(a, c).abs() > 0.03 || dot(b, c).abs() > 0.03 {
        return false;
    }
    let cross = [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
    if dot(cross, c) < 0.9 {
        return false;
    }
    let p = row(3);
    p.iter().all(|v| v.is_finite())
        && p[0].abs() <= 20000.0
        && p[1].abs() <= 20000.0
        && p[2].abs() <= 3000.0
        && p[0].abs() + p[1].abs() > 1.0
}

/// Writable, ordinary memory regions (not the card's write-combined
/// windows, which are slow to read).
fn regions() -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut at = 0usize;
    loop {
        let mut info: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: a plain query into `info`.
        let n = unsafe { VirtualQuery(at as _, &mut info, size_of::<MEMORY_BASIC_INFORMATION>()) };
        if n == 0 {
            break;
        }
        let base = info.BaseAddress as usize;
        let size = info.RegionSize;
        let protect = info.Protect;
        if info.State == MEM_COMMIT
            && protect & 0xFF == PAGE_READWRITE
            && protect & (PAGE_GUARD | PAGE_NOCACHE | PAGE_WRITECOMBINE) == 0
        {
            out.push((base, size));
        }
        let Some(next) = base.checked_add(size) else { break };
        if next <= at {
            break;
        }
        at = next;
    }
    out
}

/// Every placement matrix in writable memory: address and position.
fn scan() -> Vec<(usize, [f32; 3])> {
    let regions = regions();
    let total: usize = regions.iter().map(|r| r.1).sum();
    let started = Instant::now();
    let found = Mutex::new(Vec::new());
    let next = std::sync::atomic::AtomicUsize::new(0);
    let threads = std::thread::available_parallelism().map_or(4, |n| (n.get() / 2).clamp(1, 8));
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                let mut buffer = vec![0u8; 4 << 20];
                let mut mine = Vec::new();
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(&(base, size)) = regions.get(i) else { break };
                    let mut off = 0;
                    while off < size {
                        let len = (size - off).min(buffer.len());
                        let chunk = &mut buffer[..len];
                        if !read_bytes(base + off, chunk) {
                            off += len;
                            continue;
                        }
                        let mut o = 0;
                        while o + 64 <= len {
                            // Rows of length 1: the first float is in -1..=1.
                            let f0 = f32::from_le_bytes(chunk[o..o + 4].try_into().unwrap());
                            if f0.abs() <= 1.0 {
                                let m: [f32; 16] = std::array::from_fn(|k| {
                                    f32::from_le_bytes(chunk[o + k * 4..o + k * 4 + 4].try_into().unwrap())
                                });
                                if is_placement(&m) {
                                    mine.push((base + off + o, [m[12], m[13], m[14]]));
                                }
                            }
                            o += 16;
                        }
                        off += len;
                    }
                    if mine.len() > 1 << 16 {
                        found.lock().unwrap_or_else(|e| e.into_inner()).append(&mut mine);
                    }
                }
                found.lock().unwrap_or_else(|e| e.into_inner()).append(&mut mine);
            });
        }
    });
    let found = found.into_inner().unwrap_or_else(|e| e.into_inner());
    tracing::info!(
        regions = regions.len(),
        mb = total >> 20,
        matrices = found.len(),
        secs = started.elapsed().as_secs_f32(),
        "tour: memory scanned for placements"
    );
    found
}

/// The candidates still placements, with their new positions.
fn reread(candidates: &[(usize, [f32; 3])]) -> Vec<(usize, [f32; 3], [f32; 3])> {
    candidates
        .iter()
        .filter_map(|&(a, old)| read_matrix(a).map(|m| (a, old, [m[12], m[13], m[14]])))
        .collect()
}

fn walk(key: u8, secs: f32) {
    hold_keys(&[key]);
    std::thread::sleep(Duration::from_secs_f32(secs));
    hold_keys(&[]);
    std::thread::sleep(Duration::from_secs(2));
}

/// Finds the player's matrices: those that move with a walk forward, hold
/// still, then come back with a walk backward.
fn find_player() -> Result<Player, String> {
    let all = scan();
    if all.is_empty() {
        return Err("no placement matrix in memory".into());
    }
    let walk_secs = std::env::var("VICTORY_TOUR_WALK")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(6.0);
    let flat = |a: [f32; 3], b: [f32; 3]| ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();

    walk(b'W', walk_secs);
    let moved: Vec<(usize, [f32; 3], [f32; 3])> = reread(&all)
        .into_iter()
        .filter(|&(_, a, b)| (0.5..=60.0).contains(&flat(a, b)) && (b[2] - a[2]).abs() < 5.0)
        .collect();
    tracing::info!(moved = moved.len(), "tour: matrices that moved with the walk forward");
    // Held still: they stay.
    std::thread::sleep(Duration::from_secs(3));
    let still: Vec<(usize, [f32; 3], [f32; 3])> = moved
        .iter()
        .filter_map(|&(a, first, after)| {
            let now = read_matrix(a)?;
            let now = [now[12], now[13], now[14]];
            (flat(after, now) < 0.1).then_some((a, first, now))
        })
        .collect();
    tracing::info!(still = still.len(), "tour: of those, the ones that then stood still");
    walk(b'S', walk_secs);
    let player: Vec<usize> = still
        .iter()
        .filter_map(|&(a, first, after)| {
            let now = read_matrix(a)?;
            let now = [now[12], now[13], now[14]];
            let forward = [after[0] - first[0], after[1] - first[1]];
            let back = [now[0] - after[0], now[1] - after[1]];
            let against = forward[0] * back[0] + forward[1] * back[1] < 0.0;
            (flat(after, now) >= 0.5 && against).then_some(a)
        })
        .collect();
    tracing::info!(player = player.len(), "tour: of those, the ones that came back with the walk backward");
    for &a in player.iter().take(16) {
        let m = read_matrix(a).unwrap_or_default();
        tracing::info!(address = format_args!("{a:#x}"), position = ?[m[12], m[13], m[14]], "tour: player matrix");
    }
    if player.is_empty() {
        return Err("nothing moved with the player's walks (no control yet, or a cutscene?)".into());
    }
    // The player's body is many matrices (a skeleton's bones, its physics,
    // the camera that follows): all are written, they are recomputed from
    // the player anyway. Those far from the crowd's middle are something
    // else that happened to move alike.
    let positions: Vec<(usize, [f32; 3])> =
        player.iter().filter_map(|&a| read_matrix(a).map(|m| (a, [m[12], m[13], m[14]]))).collect();
    let median = |k: usize| {
        let mut v: Vec<f32> = positions.iter().map(|p| p.1[k]).collect();
        v.sort_by(f32::total_cmp);
        v.get(v.len() / 2).copied().unwrap_or_default()
    };
    let middle = [median(0), median(1), median(2)];
    let near: Vec<usize> = positions
        .iter()
        .filter(|(_, p)| flat(*p, middle) < 5.0 && (p[2] - middle[2]).abs() < 5.0)
        .map(|&(a, _)| a)
        .collect();
    tracing::info!(near = near.len(), ?middle, "tour: of those, the ones around the player");
    if near.is_empty() || near.len() > 4096 {
        return Err(format!("{} matrices around the player: cannot use them", near.len()));
    }
    let found = Player { addresses: near };
    // Proof: lifted 8 m for 2 s, then let go; if the player itself moved,
    // the matrices recomputed from it stay up there for a moment.
    let up = [middle[0], middle[1], middle[2] + 8.0];
    let hold = found.hold_at(up);
    std::thread::sleep(Duration::from_secs(2));
    drop(hold);
    std::thread::sleep(Duration::from_millis(300));
    let stayed = found
        .addresses
        .iter()
        .filter_map(|&a| read_matrix(a))
        .filter(|m| m[14] > middle[2] + 4.0)
        .count();
    tracing::info!(stayed, of = found.addresses.len(), "tour: matrices still up after the lift was let go");
    if stayed * 4 < found.addresses.len() {
        // Gone on with all the same: held over and over at a place, the
        // matrices may still carry the player (the results will say).
        tracing::warn!("tour: the lift did not seem to move the player; going on all the same");
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placements_are_rotations_with_a_position() {
        let mut m = [0.0f32; 16];
        m[0] = 1.0;
        m[5] = 1.0;
        m[10] = 1.0;
        m[12] = 100.0;
        m[13] = -2000.0;
        m[14] = 30.0;
        assert!(is_placement(&m));
        m[10] = -1.0; // left-handed
        assert!(!is_placement(&m));
        m[10] = 1.0;
        m[12] = 0.0;
        m[13] = 0.0;
        assert!(!is_placement(&m));
    }

    #[test]
    fn the_grid_spirals_out() {
        let g = grid([10.0, 20.0, 5.0], 100.0, 2, 40.0);
        assert_eq!(g.len(), 1 + 8 + 16);
        assert_eq!(g[0].at, [10.0, 20.0, 45.0]);
        let mut seen = std::collections::HashSet::new();
        assert!(g.iter().all(|p| seen.insert((p.at[0] as i32, p.at[1] as i32))));
    }
}
