//! A small box in the top-right corner of the game window with the frame
//! rate, the card's and the processor's use and the memory in use
//! (`VICTORY_OVERLAY=0` turns it off).
//!
//! The box is a window of its own, owned by none, kept above the game
//! window and moved with it by a thread of ours. Nothing is drawn into the
//! title's frames, so presenting pays nothing: [`frame_rate`] stores two
//! numbers once a second, and the thread reads the system's counters and
//! repaints twice a second.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::w;

/// Whether the box is shown (on unless `VICTORY_OVERLAY=0`).
fn on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_OVERLAY").is_none_or(|v| v != "0"))
}

/// The game window, the last frame rate and ms per frame (`f64` bits).
static GAME: AtomicUsize = AtomicUsize::new(0);
static FPS: AtomicU64 = AtomicU64::new(0);
static MS: AtomicU64 = AtomicU64::new(0);

/// Told once a second by the presenting code; starts the box's thread the
/// first time.
pub(crate) fn frame_rate(hwnd: HWND, fps: f64, ms: f64) {
    if !on() {
        return;
    }
    GAME.store(hwnd.0 as usize, Ordering::Relaxed);
    FPS.store(fps.to_bits(), Ordering::Relaxed);
    MS.store(ms.to_bits(), Ordering::Relaxed);
    static START: std::sync::Once = std::sync::Once::new();
    START.call_once(|| {
        let _ = std::thread::Builder::new().name("victory-overlay".into()).spawn(run);
    });
}

const WIDTH: i32 = 330;
/// The box's height: five lines of numbers, a separator and the
/// settings' lines.
fn height() -> i32 {
    98 + settings().len() as i32 * 17 + if settings().is_empty() { 0 } else { 8 }
}

/// The special settings this run has on, as the user calls them: the
/// console mode and the environment switches that change speed or the
/// picture. Read once (they are fixed for a run).
fn settings() -> &'static [&'static str] {
    static SETTINGS: OnceLock<Vec<&'static str>> = OnceLock::new();
    SETTINGS.get_or_init(|| {
        let set = |name: &str| std::env::var_os(name).is_some_and(|v| !v.is_empty() && v != "0");
        let mut out = vec![match crate::console::current() {
            crate::console::Console::SeriesS => "Mode Series S (FSR, effets bas)",
            crate::console::Console::SeriesX => "Mode Series X (qualite)",
        }];
        let switches: [(&[&str], &str); 10] = [
            (&["VICTORY_FFX_SCALE", "VICTORY_UPSCALER"], "FSR AMD (rendu plus bas)"),
            (&["VICTORY_LATE_WAIT"], "CPU et GPU en parallele"),
            (&["VICTORY_RELAY"], "Recopies sur un coeur a part"),
            (&["VICTORY_WIDE_REFRESH", "VICTORY_WIDE_BACK", "VICTORY_WIDE_MERGE"], "Copies multi-coeurs"),
            (&["VICTORY_JOIN_COPIES"], "Copies groupees"),
            (&["VICTORY_TEXTURE_NO_WAIT"], "Textures sans attente"),
            (&["VICTORY_INPLACE"], "Memoire en place"),
            (&["VICTORY_TEXTURE_HALF"], "Textures basse resolution"),
            (&["VICTORY_BVH_BUILDER"], "Rayons (RT) actifs"),
            (&["VICTORY_ROOT_UAVS_ONLY", "VICTORY_ROOT_UAVS"], "Eclairage calcule"),
        ];
        for (names, label) in switches {
            if names.iter().any(|n| set(n)) {
                out.push(label);
            }
        }
        out
    })
}
const MARGIN: i32 = 8;
const TIMER: usize = 1;

/// The text shown, made by the timer and painted by `WM_PAINT`.
static TEXT: std::sync::Mutex<Vec<u16>> = std::sync::Mutex::new(Vec::new());

fn run() {
    // SAFETY: registers our class once, creates the box and pumps its
    // messages on this thread.
    unsafe {
        let Ok(instance) = GetModuleHandleW(None) else { return };
        let class = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(window_proc),
            hInstance: instance.into(),
            lpszClassName: w!("VIctoryOverlay"),
            ..Default::default()
        };
        RegisterClassExW(&class);
        let Ok(hwnd) = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            w!("VIctoryOverlay"),
            w!("VIctory stats"),
            WS_POPUP,
            0,
            0,
            WIDTH,
            height(),
            None,
            None,
            Some(instance.into()),
            None,
        ) else {
            return;
        };
        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 200, LWA_ALPHA);
        SetTimer(Some(hwnd), TIMER, 500, None);
        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

extern "system" fn window_proc(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match message {
        WM_TIMER => {
            tick(hwnd);
            LRESULT(0)
        }
        WM_PAINT => {
            paint(hwnd);
            LRESULT(0)
        }
        // SAFETY: the default handling for our window.
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

/// Places the box over the game window's top-right corner (hidden while
/// the game is minimized or another program is in front), and remakes the
/// text.
fn tick(hwnd: HWND) {
    let game = HWND(GAME.load(Ordering::Relaxed) as *mut _);
    // SAFETY: window calls on our box and on the game's window handle.
    unsafe {
        let mut client = RECT::default();
        let shown = !game.0.is_null()
            && IsWindow(Some(game)).as_bool()
            && !IsIconic(game).as_bool()
            && in_front(game)
            && GetClientRect(game, &mut client).is_ok();
        if !shown {
            let _ = ShowWindow(hwnd, SW_HIDE);
            return;
        }
        let mut corner = POINT { x: client.right, y: client.top };
        let _ = ClientToScreen(game, &mut corner);
        // A maximized or borderless window's client area can start above
        // or right of its screen: keep the box on the screen.
        let mut screen = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        if GetMonitorInfoW(MonitorFromWindow(game, MONITOR_DEFAULTTONEAREST), &mut screen).as_bool() {
            corner.x = corner.x.min(screen.rcWork.right);
            corner.y = corner.y.max(screen.rcWork.top);
        }
        let text = stats().text();
        *TEXT.lock().unwrap_or_else(|e| e.into_inner()) = text;
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            corner.x - WIDTH - MARGIN,
            corner.y + MARGIN,
            WIDTH,
            height(),
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
        let _ = InvalidateRect(Some(hwnd), None, true);
    }
}

/// Whether the game window (or a window of this process) is the one in
/// front.
fn in_front(game: HWND) -> bool {
    // SAFETY: plain window queries.
    unsafe {
        let front = GetForegroundWindow();
        if front == game {
            return true;
        }
        let mut pid = 0;
        GetWindowThreadProcessId(front, Some(&mut pid));
        pid == std::process::id()
    }
}

fn paint(hwnd: HWND) {
    // SAFETY: the usual paint of our own window with GDI.
    unsafe {
        let mut ps = PAINTSTRUCT::default();
        let dc = BeginPaint(hwnd, &mut ps);
        let mut rect = RECT::default();
        let _ = GetClientRect(hwnd, &mut rect);
        let brush = CreateSolidBrush(COLORREF(0x0010_1010));
        FillRect(dc, &rect, brush);
        let _ = DeleteObject(brush.into());
        let font = CreateFontW(
            -15,
            0,
            0,
            0,
            FW_SEMIBOLD.0 as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            FF_DONTCARE.0 as u32,
            w!("Consolas"),
        );
        let old = SelectObject(dc, font.into());
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, COLORREF(0x0080_FF80));
        rect.left += 8;
        rect.top += 6;
        let mut text = TEXT.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if !text.is_empty() {
            DrawTextW(dc, &mut text, &mut rect, DT_LEFT | DT_TOP | DT_NOPREFIX);
        }
        SelectObject(dc, old);
        let _ = DeleteObject(font.into());
        let _ = EndPaint(hwnd, &ps);
    }
}

/// The system's counters between two ticks.
struct Stats {
    last_system: Option<[u64; 3]>,
    gpu: Option<Gpu>,
    gpu_tried: bool,
}

fn stats() -> std::sync::MutexGuard<'static, Stats> {
    static STATS: std::sync::Mutex<Stats> =
        std::sync::Mutex::new(Stats { last_system: None, gpu: None, gpu_tried: false });
    STATS.lock().unwrap_or_else(|e| e.into_inner())
}

impl Stats {
    fn text(&mut self) -> Vec<u16> {
        let fps = f64::from_bits(FPS.load(Ordering::Relaxed));
        let ms = f64::from_bits(MS.load(Ordering::Relaxed));
        let cpu = self.cpu().map_or("--".to_string(), |p| format!("{p:.0} %"));
        if !self.gpu_tried {
            self.gpu_tried = true;
            self.gpu = Gpu::open();
        }
        let (gpu, vram) = match self.gpu.as_ref().map(Gpu::read) {
            Some((busy, vram)) => (
                busy.map_or("--".to_string(), |p| format!("{p:.0} %")),
                vram.map_or("--".to_string(), |b| format!("{:.1} Go", b / 1e9)),
            ),
            None => ("--".to_string(), "--".to_string()),
        };
        let (game_ram, used_ram, total_ram) = memory();
        let text = format!(
            "FPS  {fps:.1}  ({ms:.0} ms)\nGPU  {gpu}   VRAM {vram}\nCPU  {cpu}\nRAM  jeu {:.1} Go\n     PC {:.1}/{:.0} Go",
            game_ram / 1e9,
            used_ram / 1e9,
            total_ram / 1e9,
        );
        let mut text = text;
        if !settings().is_empty() {
            text.push_str("\n------------------------");
            for label in settings() {
                text.push_str("\n+ ");
                text.push_str(label);
            }
        }
        text.encode_utf16().collect()
    }

    /// The whole PC's processor use since the last tick, in percent.
    fn cpu(&mut self) -> Option<f64> {
        use windows_sys::Win32::Foundation::FILETIME;
        use windows_sys::Win32::System::Threading::GetSystemTimes;
        let [mut idle, mut kernel, mut user] = [FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 }; 3];
        // SAFETY: three out-parameters.
        if unsafe { GetSystemTimes(&mut idle, &mut kernel, &mut user) } == 0 {
            return None;
        }
        let now = [idle, kernel, user]
            .map(|t| (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime));
        let last = self.last_system.replace(now)?;
        let [idle, kernel, user] = [0, 1, 2].map(|i| now[i].saturating_sub(last[i]) as f64);
        // Kernel time includes idle time.
        let total = kernel + user;
        (total > 0.0).then(|| ((total - idle) / total * 100.0).clamp(0.0, 100.0))
    }
}

/// Bytes: this process's private memory, the PC's memory in use, the PC's
/// memory.
fn memory() -> (f64, f64, f64) {
    use windows_sys::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX};
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    // SAFETY: zeroed plain-data structures with their sizes set.
    unsafe {
        let mut c: PROCESS_MEMORY_COUNTERS_EX = std::mem::zeroed();
        c.cb = size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
        K32GetProcessMemoryInfo(GetCurrentProcess(), (&raw mut c).cast(), c.cb);
        let mut m: MEMORYSTATUSEX = std::mem::zeroed();
        m.dwLength = size_of::<MEMORYSTATUSEX>() as u32;
        GlobalMemoryStatusEx(&mut m);
        (
            c.PrivateUsage as f64,
            m.ullTotalPhys.saturating_sub(m.ullAvailPhys) as f64,
            m.ullTotalPhys as f64,
        )
    }
}

/// The card's 3D engine use and this process's card memory, from the
/// system's performance counters (what Task Manager shows).
struct Gpu {
    query: windows_sys::Win32::System::Performance::PDH_HQUERY,
    busy: windows_sys::Win32::System::Performance::PDH_HCOUNTER,
    vram: windows_sys::Win32::System::Performance::PDH_HCOUNTER,
}

// SAFETY: the handles are only used by the box's thread, under the lock.
unsafe impl Send for Gpu {}

impl Gpu {
    fn open() -> Option<Self> {
        use windows_sys::Win32::System::Performance::*;
        let mut query = std::ptr::null_mut();
        let (mut busy, mut vram) = (std::ptr::null_mut(), std::ptr::null_mut());
        let pid = std::process::id();
        let busy_path: Vec<u16> =
            format!("\\GPU Engine(pid_{pid}_*engtype_3D)\\Utilization Percentage\0").encode_utf16().collect();
        let vram_path: Vec<u16> =
            format!("\\GPU Process Memory(pid_{pid}_*)\\Dedicated Usage\0").encode_utf16().collect();
        // SAFETY: a new query and two counters added to it.
        unsafe {
            if PdhOpenQueryW(std::ptr::null(), 0, &mut query) != 0 {
                return None;
            }
            let _ = PdhAddEnglishCounterW(query, busy_path.as_ptr(), 0, &mut busy);
            let _ = PdhAddEnglishCounterW(query, vram_path.as_ptr(), 0, &mut vram);
            PdhCollectQueryData(query);
        }
        Some(Self { query, busy, vram })
    }

    /// The 3D engine's use in percent and the card memory in bytes.
    fn read(&self) -> (Option<f64>, Option<f64>) {
        use windows_sys::Win32::System::Performance::PdhCollectQueryData;
        // SAFETY: our query.
        if unsafe { PdhCollectQueryData(self.query) } != 0 {
            return (None, None);
        }
        let busy = sum(self.busy).map(|p| p.clamp(0.0, 100.0));
        (busy, sum(self.vram))
    }
}

/// The sum over a wildcard counter's instances.
fn sum(counter: windows_sys::Win32::System::Performance::PDH_HCOUNTER) -> Option<f64> {
    use windows_sys::Win32::System::Performance::*;
    if counter.is_null() {
        return None;
    }
    let (mut size, mut count) = (0u32, 0u32);
    // SAFETY: the first call asks for the size, the second fills a buffer
    // of that size (aligned for the items it holds).
    unsafe {
        let status = PdhGetFormattedCounterArrayW(counter, PDH_FMT_DOUBLE, &mut size, &mut count, std::ptr::null_mut());
        if status != PDH_MORE_DATA || size == 0 {
            return None;
        }
        let items = (size as usize).div_ceil(size_of::<PDH_FMT_COUNTERVALUE_ITEM_W>());
        let mut buffer: Vec<PDH_FMT_COUNTERVALUE_ITEM_W> = Vec::with_capacity(items);
        if PdhGetFormattedCounterArrayW(counter, PDH_FMT_DOUBLE, &mut size, &mut count, buffer.as_mut_ptr()) != 0 {
            return None;
        }
        buffer.set_len((count as usize).min(items));
        Some(
            buffer
                .iter()
                .filter(|i| i.FmtValue.CStatus == 0)
                .map(|i| i.FmtValue.Anonymous.doubleValue)
                .sum(),
        )
    }
}
