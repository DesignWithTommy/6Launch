//! Frame-rate overlay for every runtime: Vice-Tay's HUD (src/hud.rs, copied
//! from TaynanGT/Vice-Tay with credit) in a topmost, click-through window over
//! the game's top-left corner. It is fed from the log lines the launcher
//! already reads (the victory family's "12.3 fps, 81.3 ms/frame ... N
//! draws/frame" and TuxBox's heartbeat "completed PresentX #N (12.3 fps") and
//! from Windows' counters for the runtime's process. The runtimes' own
//! overlays are switched off (runtimes.rs), so this is the one on screen.
//! F3 shows or hides it; on at start unless NEONBRIDGE_FPS=0.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::hud;

pub const WIDTH: i32 = hud::BASE_W as i32;
pub const HEIGHT: i32 = hud::BASE_H as i32;
/// Samples the graph keeps (one per log line).
const KEEP: usize = 172;

/// One frame-rate log line.
#[derive(Clone, Default)]
struct Sample {
    fps: f64,
    ms: f64,
    draws: Option<u64>,
    dropped: Option<u64>,
    gpu_ms: Option<f64>,
    preparing_ms: Option<f64>,
    detail: Option<String>,
}

/// Samples, oldest first.
static SAMPLES: Mutex<Vec<Sample>> = Mutex::new(Vec::new());
/// What the window paints, rebuilt four times a second.
static CANVAS: Mutex<Option<hud::Canvas>> = Mutex::new(None);
static SHOWN: AtomicBool = AtomicBool::new(false);
static STARTED: AtomicBool = AtomicBool::new(false);

/// Whether the box is on screen now (the controls panel moves below it).
pub fn shown() -> bool {
    SHOWN.load(Ordering::Relaxed)
}

/// (fps, ms/frame) of a frame-rate or heartbeat line.
fn parse(line: &str) -> Option<(f64, f64)> {
    let num = |s: &str| s.rsplit([' ', '(']).next()?.parse::<f64>().ok().filter(|v| v.is_finite() && *v >= 0.0);
    let fps = if let Some(at) = line.find(" fps, ") {
        num(&line[..at])?
    } else {
        let rest = line.split("completed PresentX #").nth(1)?;
        let at = rest.find(" fps")?;
        num(&rest[..at])?
    };
    let ms = line
        .split(" ms/frame")
        .next()
        .filter(|head| head.len() < line.len())
        .and_then(|head| head.rsplit([' ', ',']).next()?.parse::<f64>().ok())
        .unwrap_or(if fps > 0.0 { 1000.0 / fps } else { 0.0 });
    Some((fps, ms))
}

/// The number right before `key` in `line` ("1262 draws/frame").
fn before(line: &str, key: &str) -> Option<f64> {
    let at = line.find(key)?;
    line[..at].trim_end().rsplit([' ', '(', ',']).next()?.parse::<f64>().ok().filter(|v| v.is_finite())
}

/// The number right after `key` in `line` ("copies 30.6 ms").
fn after(line: &str, key: &str) -> Option<f64> {
    line.split(key).nth(1)?.split(' ').next()?.parse::<f64>().ok().filter(|v| v.is_finite())
}

/// Takes one log line; frame-rate lines become samples.
pub fn feed(line: &str) {
    let Some((fps, ms)) = parse(line) else { return };
    let preparing_ms = before(line, " preparing");
    let detail = preparing_ms.map(|p| format!("PREP {p:.1} | COPIES {:.1} | TEX {:.1}", after(line, "copies ").unwrap_or(0.0), after(line, "textures ").unwrap_or(0.0)));
    let s = Sample {
        fps,
        ms,
        draws: before(line, " draws/frame").map(|v| v as u64),
        dropped: after(line, "dropped ").map(|v| v as u64),
        gpu_ms: before(line, " on the GPU"),
        preparing_ms,
        detail,
    };
    let mut v = SAMPLES.lock().unwrap_or_else(|e| e.into_inner());
    v.push(s);
    if v.len() > KEEP {
        v.remove(0);
    }
}

/// The panel's numbers now.
fn snapshot(runtime: &str, game: HWND, watch: &Option<hud::GpuWatch>, cpu: &hud::CpuMeter) -> hud::Snapshot {
    let samples = SAMPLES.lock().map(|v| v.clone()).unwrap_or_default();
    let last = samples.last().cloned().unwrap_or_default();
    let (gpu_name, vram_total) = hud::gpu_info().clone();
    let (load, dedicated, shared) = watch.as_ref().map_or((0.0, 0.0, 0.0), |w| w.sample());
    let pid = watch.as_ref().map_or(0, |w| w.pid());
    let (used, total, commit, _) = hud::system_memory();
    let mut client: RECT = unsafe { std::mem::zeroed() };
    unsafe { GetClientRect(game, &mut client) };
    let gb = |b: f64| b / (1u64 << 30) as f64;
    let recent = &samples[samples.len().saturating_sub(100)..];
    let low = recent.iter().map(|s| s.fps).fold(f64::INFINITY, f64::min);
    hud::Snapshot {
        fps: last.fps,
        frame_ms: last.ms,
        gpu_ms: last.gpu_ms.unwrap_or(0.0),
        gpu_pct: load.round() as u32,
        gpu_measured: watch.is_some(),
        submit_ms: last.preparing_ms.unwrap_or(0.0),
        low1_fps: if low.is_finite() { low } else { 0.0 },
        max_ms: recent.iter().map(|s| s.ms).fold(0.0, f64::max),
        draws: last.draws.unwrap_or(0),
        dropped: last.dropped.unwrap_or(0),
        width: client.right.max(0) as u32,
        height: client.bottom.max(0) as u32,
        gpu_name,
        vram_used_gb: gb(dedicated),
        vram_budget_gb: gb(vram_total as f64),
        vram_shared_gb: gb(shared),
        vram_rt_gb: 0.0,
        vram_tex_gb: 0.0,
        vram_buf_gb: 0.0,
        vram_bvh_gb: 0.0,
        vram_oth_gb: 0.0,
        cpu_name: hud::cpu_name().to_owned(),
        cpu_pct: cpu.total_pct,
        cores_pct: cpu.cores_pct.clone(),
        proc_ram_gb: gb(hud::process_private_bytes(pid) as f64),
        sys_ram_used_gb: gb(used as f64),
        sys_ram_total_gb: gb(total as f64),
        sys_commit_gb: gb(commit as f64),
        history_ms: samples.iter().map(|s| s.ms as f32).collect(),
        runtime: runtime.to_owned(),
        detail: last.detail.unwrap_or_default(),
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Puts the box over the windows of the process named `name` (lowercase
/// file name) for the rest of the run. `window` finds the game's window.
pub fn show(name: String, window: fn(&str) -> Option<HWND>) {
    if STARTED.swap(true, Ordering::Relaxed) {
        return;
    }
    let _ = std::thread::Builder::new().name("neonbridge fps".into()).spawn(move || run(name, window));
}

fn run(name: String, window: fn(&str) -> Option<HWND>) {
    unsafe {
        let instance = GetModuleHandleW(std::ptr::null());
        let class = wide("i95.Fps");
        let wc = WNDCLASSW { lpfnWndProc: Some(proc_), hInstance: instance, lpszClassName: class.as_ptr(), ..std::mem::zeroed() };
        RegisterClassW(&wc);
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE,
            class.as_ptr(),
            class.as_ptr(),
            WS_POPUP,
            0,
            0,
            WIDTH,
            HEIGHT,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            return;
        }
        SetLayeredWindowAttributes(hwnd, 0, 235, LWA_ALPHA);
        let mut wanted = std::env::var("NEONBRIDGE_FPS").map_or(true, |v| v.trim() != "0");
        let mut f3 = false;
        let mut watch: Option<hud::GpuWatch> = None;
        let mut cpu = hud::CpuMeter::default();
        let mut painted = std::time::Instant::now() - hud::EVERY;
        let label = name.trim_end_matches(".exe").to_owned();
        let mut msg: MSG = std::mem::zeroed();
        loop {
            while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            let game = window(&name);
            let focused = game.is_some_and(|g| GetForegroundWindow() == g);
            let f3_now = GetAsyncKeyState(0x72) as u16 & 0x8000 != 0;
            if focused && f3_now && !f3 {
                wanted = !wanted;
            }
            f3 = f3_now;
            let show = wanted && crate::splash::closed() && focused;
            if let (true, Some(g)) = (show, game) {
                // Top-left of the picture, where Vice-Tay draws it.
                let mut origin = POINT { x: 0, y: 0 };
                ClientToScreen(g, &mut origin);
                SetWindowPos(hwnd, HWND_TOPMOST, origin.x, origin.y, 0, 0, SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW);
                if painted.elapsed() >= hud::EVERY || !SHOWN.load(Ordering::Relaxed) {
                    painted = std::time::Instant::now();
                    let mut pid = 0;
                    GetWindowThreadProcessId(g, &mut pid);
                    if watch.as_ref().is_none_or(|w| w.pid() != pid) {
                        watch = hud::GpuWatch::new(pid);
                    }
                    cpu.sample();
                    let snap = snapshot(&label, g, &watch, &cpu);
                    *CANVAS.lock().unwrap_or_else(|e| e.into_inner()) = Some(hud::paint_hud(&snap));
                    InvalidateRect(hwnd, std::ptr::null(), 0);
                }
            } else if SHOWN.load(Ordering::Relaxed) {
                ShowWindow(hwnd, SW_HIDE);
            }
            SHOWN.store(show, Ordering::Relaxed);
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
}

unsafe extern "system" fn proc_(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe {
        if msg != WM_PAINT {
            return DefWindowProcW(hwnd, msg, w, l);
        }
        let mut ps: PAINTSTRUCT = std::mem::zeroed();
        let dc = BeginPaint(hwnd, &mut ps);
        if let Some(c) = CANVAS.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            let bgra: Vec<u32> = c.rgb.iter().map(|&[r, g, b]| (r as u32) << 16 | (g as u32) << 8 | b as u32).collect();
            let mut info: BITMAPINFO = std::mem::zeroed();
            info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
            info.bmiHeader.biWidth = c.w as i32;
            info.bmiHeader.biHeight = -(c.h as i32);
            info.bmiHeader.biPlanes = 1;
            info.bmiHeader.biBitCount = 32;
            info.bmiHeader.biCompression = BI_RGB;
            SetDIBitsToDevice(dc, 0, 0, c.w as u32, c.h as u32, 0, 0, 0, c.h as u32, bgra.as_ptr().cast(), &info, DIB_RGB_COLORS);
        }
        EndPaint(hwnd, &ps);
        0
    }
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn victory_and_tuxbox_lines() {
        assert_eq!(parse("[victory] 12.5 fps, 80.0 ms/frame (gpu 3.1)"), Some((12.5, 80.0)));
        assert_eq!(parse("[tuxbox] heartbeat: completed PresentX #120 (25.0 fps)"), Some((25.0, 40.0)));
        assert_eq!(parse("completed PresentX #9 (2.5 fps; ms/frame: record 1, close 2)"), Some((2.5, 400.0)));
        assert_eq!(parse("nothing here"), None);
    }
}
