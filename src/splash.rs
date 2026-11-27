//! Loading screen shown from launch until the game presents its first frame.
//!
//! A borderless, topmost window on its own thread with a status line and a
//! progress bar. The launcher moves it through its stages with [`status`];
//! [`close`] hides it when the title's first PresentX completes (or it exits).
//! NEONBRIDGE_SPLASH=0 turns it off.
//!
//! [`watch`] stops the title's first, unpainted window flashing white across
//! both monitors: titles create it at 3840x2160 and only the swap chain used
//! to fit it. The watcher fits it into one monitor's work area as soon as it
//! exists (xmem keeps it cloaked until the first frame, so no curtain is
//! drawn over it).

use std::sync::Mutex;
use std::sync::atomic::{AtomicIsize, Ordering};
use windows_sys::core::BOOL;
use windows_sys::Win32::Foundation::{CloseHandle, COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW};
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const WIDTH: i32 = 560;
const HEIGHT: i32 = 214;
/// The card's close button, top-right.
const CLOSE_BOX: RECT = RECT { left: WIDTH - 30, top: 2, right: WIDTH - 2, bottom: 30 };
const BACKGROUND: COLORREF = rgb(11, 11, 20);
const TRACK: COLORREF = rgb(32, 30, 48);
/// The pulse: a faint sheen moving through the unfilled track.
const SHEEN: COLORREF = rgb(52, 56, 78);
const PINK: COLORREF = rgb(255, 46, 151);
const CYAN: COLORREF = rgb(0, 229, 255);
const TEXT: COLORREF = rgb(200, 200, 220);

const fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    r as u32 | (g as u32) << 8 | (b as u32) << 16
}

struct State {
    /// Where the bar should be, 0..1; it only grows.
    target: f32,
    /// The step prints no count: an indeterminate pulse runs past the bar's
    /// end instead of a made-up percentage.
    pulse: bool,
    /// When the current step began, for its elapsed time while it pulses.
    since: Option<std::time::Instant>,
    /// Drawn position; eases towards the target and never moves back.
    shown: f32,
    text: String,
    tick: u32,
}

static STATE: Mutex<State> =
    Mutex::new(State { target: 0.0, pulse: true, since: None, shown: 0.0, text: String::new(), tick: 0 });
static WINDOW: AtomicIsize = AtomicIsize::new(0);

fn state() -> std::sync::MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Opens the loading screen (once; no-op when disabled).
pub fn show(text: &str) {
    if std::env::var("NEONBRIDGE_SPLASH").as_deref() == Ok("0") || WINDOW.load(Ordering::Acquire) != 0 {
        return;
    }
    state().text = text.to_owned();
    WINDOW.store(-1, Ordering::Release);
    let _ = std::thread::Builder::new().name("neonbridge splash".into()).spawn(run);
}

/// Moves the bar to stage start `progress` (0..1) with a new status line.
pub fn status(progress: f32, text: &str) {
    let mut s = state();
    s.target = s.target.max(progress.clamp(0.0, 1.0));
    s.pulse = true;
    // A new step (not a recount of the same one) restarts its clock.
    let head = |t: &str| t.split(": ").next().unwrap_or("").to_owned();
    if s.since.is_none() || head(&s.text) != head(text) {
        s.since = Some(std::time::Instant::now());
    }
    s.text = text.to_owned();
}

/// A step with no count to show: the bar stays where it is and pulses.
pub fn pulse(text: &str) {
    let mut s = state();
    s.pulse = true;
    if s.text != text {
        s.text = text.to_owned();
    }
}

/// One step's count on its share of the bar (`from`..`to`): "label: 1,000 /
/// 26,503", or with no total "label: 1,000" and a pulse.
pub fn step(from: f32, to: f32, label: &str, done: u64, total: Option<u64>) {
    match total.filter(|&t| t > 0) {
        Some(t) => counted(from + (to - from) * (done.min(t) as f32 / t as f32), &format!("{label}: {} / {}", crate::progress::grouped(done.min(t)), crate::progress::grouped(t))),
        None => {
            status(from, &format!("{label}: {}", crate::progress::grouped(done)));
        }
    }
}

/// Moves the bar to `progress` (0..1) from real completed/total counts, with
/// `text` (the meter line, its time left). Cheap: the window redraws on its
/// own timer, whatever the call rate.
pub fn counted(progress: f32, text: &str) {
    if WINDOW.load(Ordering::Acquire) == 0 {
        return;
    }
    let mut s = state();
    s.target = s.target.max(progress.clamp(0.0, 1.0));
    s.pulse = false;
    if s.text != text {
        s.text = text.to_owned();
    }
}

/// The loading screen is gone (closed, or never shown).
pub fn closed() -> bool {
    matches!(WINDOW.load(Ordering::Acquire), 0 | -2)
}

/// Closes the loading screen.
pub fn close() {
    let hwnd = WINDOW.swap(-2, Ordering::AcqRel);
    if hwnd > 0 {
        unsafe { PostMessageW(hwnd as HWND, WM_CLOSE, 0, 0) };
    }
}

/// Fits the windows of the game process `exe` into the primary monitor's work
/// area from the moment they exist and for the rest of the run, and covers
/// them with the loading screen until [`close`]. Runs on its own thread and polls every few milliseconds, so
/// a window spanning both monitors moves before it is ever painted.
pub fn watch(exe: &std::path::Path) {
    let Some(name) = exe.file_name().map(|n| n.to_string_lossy().to_lowercase()) else { return };
    let _ = std::thread::Builder::new().name("neonbridge window watch".into()).spawn(move || {
        let mut names: std::collections::HashMap<u32, bool> = std::collections::HashMap::new();
        let mut game: HWND = std::ptr::null_mut();
        let mut focused = false;
        // Runs for the whole game: the title stretches its window over every
        // monitor again later, so it is pulled back onto one each time.
        loop {
            let closed = WINDOW.load(Ordering::Acquire) == -2;
            if closed && !focused {
                focused = true;
                // The title only reads input while it has focus (the auto A press
                // for its player-selection prompt included), so hand it the foreground.
            }
            for hwnd in top_level_windows() {
                let mut pid = 0;
                unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
                if !*names.entry(pid).or_insert_with(|| image_name(pid).is_some_and(|n| n == name)) {
                    continue;
                }
                if fit(hwnd).is_some() {
                    game = hwnd;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(if closed { 50 } else { 4 }));
        }
    });
}

fn rect_eq(a: &RECT, b: &RECT) -> bool {
    (a.left, a.top, a.right, a.bottom) == (b.left, b.top, b.right, b.bottom)
}

fn top_level_windows() -> Vec<HWND> {
    unsafe extern "system" fn visit(hwnd: HWND, out: LPARAM) -> BOOL {
        unsafe { (*(out as *mut Vec<HWND>)).push(hwnd) };
        1
    }
    let mut out: Vec<HWND> = Vec::new();
    unsafe { EnumWindows(Some(visit), &mut out as *mut Vec<HWND> as LPARAM) };
    out
}

fn image_name(pid: u32) -> Option<String> {
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return None;
        }
        let mut buffer = [0u16; 1024];
        let mut len = buffer.len() as u32;
        let ok = QueryFullProcessImageNameW(process, 0, buffer.as_mut_ptr(), &mut len) != 0;
        CloseHandle(process);
        if !ok {
            return None;
        }
        let path = String::from_utf16_lossy(&buffer[..len as usize]);
        Some(path.rsplit('\\').next().unwrap_or_default().to_lowercase())
    }
}

/// Moves a title window that leaves its monitor's work area into the primary
/// monitor's at 16:9, and returns its bounds once visible.
/// Tool, owned and child windows are left alone (None), as is a window that is
/// already fullscreen on, or inside, one monitor.
fn fit(hwnd: HWND) -> Option<RECT> {
    unsafe {
        let extended = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
        if extended & WS_EX_TOOLWINDOW != 0 || style & WS_CHILD != 0 || !GetWindow(hwnd, GW_OWNER).is_null() {
            return None;
        }
        let mut current: RECT = std::mem::zeroed();
        let mut client: RECT = std::mem::zeroed();
        if GetWindowRect(hwnd, &mut current) == 0 || GetClientRect(hwnd, &mut client) == 0 {
            return None;
        }
        let visible = IsWindowVisible(hwnd) != 0;
        // A maximised window overhangs its work area by its frame; leave it.
        if IsZoomed(hwnd) != 0 {
            return visible.then_some(current);
        }
        let mut mine: MONITORINFO = std::mem::zeroed();
        mine.cbSize = size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST), &mut mine) != 0
            && (rect_eq(&current, &mine.rcMonitor)
                || (current.left >= mine.rcWork.left && current.top >= mine.rcWork.top && current.right <= mine.rcWork.right && current.bottom <= mine.rcWork.bottom))
        {
            return visible.then_some(current);
        }
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY), &mut info) == 0 {
            return None;
        }
        let work = info.rcWork;
        let mut frame: RECT = std::mem::zeroed();
        AdjustWindowRectEx(&mut frame, style, (!GetMenu(hwnd).is_null()).into(), extended);
        let (border_w, border_h) = (frame.right - frame.left, frame.bottom - frame.top);
        // The title sizes this window to the whole desktop (32:9 across two
        // monitors); its swap chain is 16:9 (3840x2160), so fit that shape.
        let _ = client;
        let (w, h) = (16i64, 9i64);
        let available_w = i64::from(work.right - work.left - border_w);
        let available_h = i64::from(work.bottom - work.top - border_h);
        if available_w <= 0 || available_h <= 0 {
            return None;
        }
        let client_w = available_w.min(available_h * w / h).max(1);
        let client_h = (client_w * h / w).max(1);
        let (outer_w, outer_h) = (client_w as i32 + border_w, client_h as i32 + border_h);
        let left = work.left + (work.right - work.left - outer_w) / 2;
        let top = work.top + (work.bottom - work.top - outer_h) / 2;
        SetWindowPos(hwnd, std::ptr::null_mut(), left, top, outer_w, outer_h, SWP_NOZORDER | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS);
        visible.then_some(RECT { left, top, right: left + outer_w, bottom: top + outer_h })
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain([0]).collect()
}

fn run() {
    unsafe {
        let instance = GetModuleHandleW(std::ptr::null());
        let class = wide("i95.Splash");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(proc_),
            hInstance: instance,
            hCursor: LoadCursorW(std::ptr::null_mut(), IDC_APPSTARTING),
            lpszClassName: class.as_ptr(),
            ..std::mem::zeroed()
        };
        RegisterClassW(&wc);
        let (sw, sh) = (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN));
        let title = wide("i95");
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            class.as_ptr(),
            title.as_ptr(),
            WS_POPUP | WS_VISIBLE,
            (sw - WIDTH) / 2,
            (sh - HEIGHT) / 2,
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
        // close() may have run before the window existed.
        if WINDOW.compare_exchange(-1, hwnd as isize, Ordering::AcqRel, Ordering::Acquire).is_err() {
            DestroyWindow(hwnd);
            return;
        }
        SetTimer(hwnd, 1, 33, None);
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

unsafe extern "system" fn proc_(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_TIMER => {
                {
                    let mut s = state();
                    s.tick = s.tick.wrapping_add(1);
                    // Ease towards the target (with a floor on the step, so it
                    // arrives instead of approaching forever). Only real counts
                    // move it; a step without them pulses. Never backwards.
                    if s.shown < s.target {
                        let step = ((s.target - s.shown) * 0.15).max(0.0015);
                        s.shown = (s.shown + step).min(s.target);
                    }
                }
                InvalidateRect(hwnd, std::ptr::null(), 0);
                0
            }
            WM_PAINT => {
                paint(hwnd);
                0
            }
            WM_ERASEBKGND => 1,
            // The close button, or Esc: quits i95 (the card has no caption, so no X of its own).
            WM_LBUTTONDOWN => {
                let (x, y) = ((l & 0xFFFF) as i16 as i32, ((l >> 16) & 0xFFFF) as i16 as i32);
                let mut client: RECT = std::mem::zeroed();
                GetClientRect(hwnd, &mut client);
                let (ox, oy) = ((client.right - WIDTH).max(0) / 2, (client.bottom - HEIGHT).max(0) / 2);
                let (x, y) = (x - ox, y - oy);
                if x >= CLOSE_BOX.left && x < CLOSE_BOX.right && y >= CLOSE_BOX.top && y < CLOSE_BOX.bottom {
                    std::process::exit(0);
                }
                0
            }
            WM_KEYDOWN if w == 0x1B => std::process::exit(0),
            WM_CLOSE => {
                DestroyWindow(hwnd);
                0
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                0
            }
            _ => DefWindowProcW(hwnd, msg, w, l),
        }
    }
}

unsafe fn fill(dc: HDC, r: RECT, color: COLORREF) {
    unsafe {
        let brush = CreateSolidBrush(color);
        FillRect(dc, &r, brush);
        DeleteObject(brush);
    }
}

unsafe fn font_italic(height: i32, weight: i32) -> HFONT {
    let face = wide("Segoe UI Black");
    unsafe { CreateFontW(height, 0, 0, 0, weight, 1, 0, 0, DEFAULT_CHARSET as u32, 0, 0, CLEARTYPE_QUALITY as u32, 0, face.as_ptr()) }
}

unsafe fn font(height: i32, weight: i32) -> HFONT {
    let face = wide("Segoe UI");
    unsafe { CreateFontW(height, 0, 0, 0, weight, 0, 0, 0, DEFAULT_CHARSET as u32, 0, 0, CLEARTYPE_QUALITY as u32, 0, face.as_ptr()) }
}

unsafe fn text(dc: HDC, s: &str, mut r: RECT, color: COLORREF, f: HFONT, flags: u32) {
    let mut w: Vec<u16> = s.encode_utf16().collect();
    unsafe {
        let old = SelectObject(dc, f);
        SetTextColor(dc, color);
        DrawTextW(dc, w.as_mut_ptr(), w.len() as i32, &mut r, flags | DT_SINGLELINE | DT_NOPREFIX);
        SelectObject(dc, old);
    }
}

unsafe fn paint(hwnd: HWND) {
    let (shown, line, tick, pulse, since) = {
        let s = state();
        (s.shown, s.text.clone(), s.tick, s.pulse, s.since)
    };
    unsafe {
        let mut ps: PAINTSTRUCT = std::mem::zeroed();
        let screen = BeginPaint(hwnd, &mut ps);
        // Once watch() has stretched the window into a curtain over the title,
        // the card sits centred on plain background.
        let mut client: RECT = std::mem::zeroed();
        GetClientRect(hwnd, &mut client);
        let (ox, oy) = ((client.right - WIDTH).max(0) / 2, (client.bottom - HEIGHT).max(0) / 2);
        if ox > 0 || oy > 0 {
            fill(screen, client, BACKGROUND);
        }
        // Double-buffered to avoid flicker at 30 Hz.
        let dc = CreateCompatibleDC(screen);
        let bitmap = CreateCompatibleBitmap(screen, WIDTH, HEIGHT);
        let old_bitmap = SelectObject(dc, bitmap);
        SetBkMode(dc, TRANSPARENT as i32);

        // Sunset over the coast: night purple at the top through magenta to
        // orange at the horizon, in 2 px bands.
        let mix = |a: (f32, f32, f32), b: (f32, f32, f32), t: f32| rgb((a.0 + (b.0 - a.0) * t) as u8, (a.1 + (b.1 - a.1) * t) as u8, (a.2 + (b.2 - a.2) * t) as u8);
        let (night, dusk, glow) = ((18.0, 8.0, 38.0), (92.0, 18.0, 96.0), (255.0, 122.0, 60.0));
        let mut y = 0;
        while y < HEIGHT {
            let t = (y as f32 / 138.0).min(1.0);
            let c = if t < 0.55 { mix(night, dusk, t / 0.55) } else { mix(dusk, glow, (t - 0.55) / 0.45) };
            fill(dc, RECT { left: 0, top: y, right: WIDTH, bottom: y + 2 }, c);
            y += 2;
        }
        // A striped setting sun behind the wordmark.
        // It sets as the load goes: high at the start, on the horizon at the end.
        let (cx, r) = (WIDTH - 120, 62);
        let cy = 78 + (52.0 * shown.clamp(0.0, 1.0)) as i32;
        let sun = CreateSolidBrush(rgb(255, 196, 70));
        let no_pen = GetStockObject(NULL_PEN);
        let (ob, op) = (SelectObject(dc, sun), SelectObject(dc, no_pen));
        Ellipse(dc, cx - r, cy - r, cx + r, cy + r);
        SelectObject(dc, ob);
        SelectObject(dc, op);
        DeleteObject(sun);
        // The stripes slide down through the sun, thickening as they go.
        let slide = (tick as i32 / 2) % 10;
        for i in 0..6 {
            let top = cy - 10 + i * 10 + slide;
            let gap = 1 + (top - (cy - 10)) / 12;
            if top + gap > cy + r {
                break;
            }
            let t = (top as f32 / 138.0).min(1.0);
            fill(dc, RECT { left: cx - r, top, right: cx + r, bottom: top + gap }, mix(dusk, glow, ((t - 0.55) / 0.45).clamp(0.0, 1.0)));
        }
        // Dark ground under the card's text.
        fill(dc, RECT { left: 0, top: 138, right: WIDTH, bottom: HEIGHT }, rgb(14, 6, 26));
        fill(dc, RECT { left: 0, top: 138, right: WIDTH, bottom: 140 }, PINK);

        // The wordmark: heavy italic, white over a hot-pink drop shadow.
        let title = font_italic(64, 900);
        let small = font(17, 600);
        text(dc, "i95", RECT { left: 36, top: 26, right: WIDTH, bottom: 104 }, rgb(255, 46, 151), title, DT_LEFT | DT_VCENTER);
        text(dc, "i95", RECT { left: 32, top: 22, right: WIDTH, bottom: 100 }, rgb(255, 255, 255), title, DT_LEFT | DT_VCENTER);
        // Counted lines carry their own numbers; dots only for waits.
        // No count: dots and the step's elapsed time ("Warming the shaders... 1m 20s").
        let dots = match since.map(|t| t.elapsed()).filter(|_| pulse) {
            Some(e) if e.as_secs() >= 1 => format!("{:<3} {}", ".".repeat((tick / 12 % 4) as usize), crate::progress::clock(e)),
            Some(_) => ".".repeat((tick / 12 % 4) as usize),
            None => String::new(),
        };
        text(dc, &format!("{line}{dots}"), RECT { left: 32, top: 146, right: WIDTH - 84, bottom: 170 }, TEXT, small, DT_LEFT | DT_VCENTER | DT_END_ELLIPSIS);

        // Progress: a road. Asphalt the whole way, the lane line painted as far
        // as the load has come, and a car at the front of it.
        let road = RECT { left: 32, top: 174, right: WIDTH - 32, bottom: 196 };
        fill(dc, road, rgb(34, 30, 44));
        fill(dc, RECT { bottom: road.top + 2, ..road }, rgb(150, 140, 170));
        fill(dc, RECT { top: road.bottom - 2, ..road }, rgb(150, 140, 170));
        let filled = road.left + ((road.right - road.left) as f32 * shown.clamp(0.0, 1.0)) as i32;
        let mid = (road.top + road.bottom) / 2;
        // Dashes scroll while it waits, as if driving.
        let shift = if pulse { (tick as i32 * 2) % 24 } else { 0 };
        let mut x = road.left - shift;
        while x < filled {
            let (l, r) = (x.max(road.left), (x + 14).min(filled));
            if r > l {
                fill(dc, RECT { left: l, top: mid - 1, right: r, bottom: mid + 2 }, rgb(255, 200, 60));
            }
            x += 24;
        }
        // The car drifts down the line (a manji): it swings nose-left,
        // nose-right across the lane, tail out, with tyre smoke behind.
        let phase = tick as f32 / 9.0;
        let angle = phase.sin() * 0.45;
        let (ccx, ccy) = ((filled - 13).max(road.left + 13) as f32, mid as f32 + phase.sin() * 3.0);
        let quad = |l: f32, t: f32, r: f32, b: f32| -> [windows_sys::Win32::Foundation::POINT; 4] {
            let (sa, ca) = (angle.sin(), angle.cos());
            [(l, t), (r, t), (r, b), (l, b)].map(|(x, y)| windows_sys::Win32::Foundation::POINT { x: (ccx + x * ca - y * sa) as i32, y: (ccy + x * sa + y * ca) as i32 })
        };
        let poly = |pts: &[windows_sys::Win32::Foundation::POINT], c: COLORREF| {
            let b = CreateSolidBrush(c);
            let (ob, op) = (SelectObject(dc, b), SelectObject(dc, GetStockObject(NULL_PEN)));
            Polygon(dc, pts.as_ptr(), pts.len() as i32);
            SelectObject(dc, ob);
            SelectObject(dc, op);
            DeleteObject(b);
        };
        // Smoke: a few grey puffs trailing the tail, swinging opposite.
        for k in 1..5 {
            let px = ccx as i32 - 14 - k * 7;
            if px < road.left {
                break;
            }
            let py = mid - (phase.sin() * 4.0) as i32 + ((phase * 2.0 + k as f32).sin() * 2.0) as i32;
            let g = (150 - k * 22) as u8;
            let rr = 5 - k / 2;
            fill(dc, RECT { left: px - rr, top: (py - rr).max(road.top + 2), right: px + rr, bottom: (py + rr).min(road.bottom - 2) }, rgb(g, g, g + 10));
        }
        poly(&quad(-13.0, -6.0, 13.0, 6.0), PINK);
        poly(&quad(3.0, -4.0, 8.0, 4.0), CYAN);
        poly(&quad(-13.0, -6.0, -11.0, -3.0), rgb(255, 60, 60));
        poly(&quad(-13.0, 3.0, -11.0, 6.0), rgb(255, 60, 60));
        text(dc, &format!("{}%", (shown * 100.0).floor() as u32), RECT { left: 32, top: 146, right: WIDTH - 32, bottom: 170 }, TEXT, small, DT_RIGHT | DT_VCENTER);

        // The close button (the card is a captionless popup): top-right, quits i95.
        text(dc, "\u{2715}", CLOSE_BOX, TEXT, small, DT_CENTER | DT_VCENTER);

        BitBlt(screen, ox, oy, WIDTH, HEIGHT, dc, 0, 0, SRCCOPY);
        SelectObject(dc, old_bitmap);
        DeleteObject(bitmap);
        DeleteDC(dc);
        DeleteObject(title);
        DeleteObject(small);
        EndPaint(hwnd, &ps);
    }
}
