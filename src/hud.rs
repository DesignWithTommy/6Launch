//! The frame-rate panel every runtime gets: Vice-Tay's HUD (TaynanGT/Vice-Tay,
//! crates/victory-gpu/src/host/panel.rs, GPL-3.0, copied with credit), painted
//! by i95 into its own click-through window over the game instead of onto the
//! swap chain, so it works the same over every runtime. Its numbers come from
//! the runtime's log lines (fps, ms/frame, draws, dropped draws, time on the
//! GPU) and from Windows (GPU load and memory per process via PDH, the
//! runtime's RAM, system RAM, per-core CPU).

#![allow(dead_code)]

use std::sync::OnceLock;
use std::time::Duration;

/// How often the overlay redraws its buffer.
pub const EVERY: Duration = Duration::from_millis(250);

/// Base 5x7 glyph bitmap size.
const GLYPH: (u32, u32) = (5, 7);

/// Logical canvas dimensions (1x at <=1600p, 2x at 4K).
pub const BASE_W: u32 = 540;
pub const BASE_H: u32 = 334;
/// Mapped buffers rotated as the queue finishes reading them.
const UPLOADS: usize = 4;

// Palette (sRGB 0..255).
const BG: [u8; 3] = [18, 20, 25];
const BORDER: [u8; 3] = [56, 62, 74];
const GRAPH_BG: [u8; 3] = [11, 13, 17];
const GRAPH_GRID: [u8; 3] = [44, 48, 58];
const GRAPH_FLOOR: [u8; 3] = [112, 62, 34];
const BAR_BG: [u8; 3] = [34, 38, 46];
const BAR_FILL_LO: [u8; 3] = [88, 212, 232];
const BAR_FILL_HI: [u8; 3] = [116, 178, 252];
const LABEL: [u8; 3] = [108, 178, 252];
const WHITE: [u8; 3] = [240, 243, 248];
const MUTED: [u8; 3] = [170, 178, 192];
const SEP: [u8; 3] = [96, 104, 118];
const GREEN: [u8; 3] = [138, 236, 72];
const YELLOW: [u8; 3] = [245, 208, 66];
const ORANGE: [u8; 3] = [246, 134, 54];
const RED: [u8; 3] = [242, 76, 66];

/// Seven rows of five dots, the leftmost the highest bit.
#[rustfmt::skip]
const fn glyph(c: u8) -> [u8; 7] {
    match c {
        b'0' => [0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110],
        b'1' => [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
        b'2' => [0b01110, 0b10001, 0b00001, 0b00110, 0b01000, 0b10000, 0b11111],
        b'3' => [0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110],
        b'4' => [0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010],
        b'5' => [0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110],
        b'6' => [0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110],
        b'7' => [0b11111, 0b00001, 0b00010, 0b00010, 0b00100, 0b00100, 0b00100],
        b'8' => [0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110],
        b'9' => [0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100],
        b'A' => [0b00100, 0b01010, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001],
        b'B' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110],
        b'C' => [0b01111, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b01111],
        b'D' => [0b11110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11110],
        b'E' => [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111],
        b'F' => [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000],
        b'G' => [0b01111, 0b10000, 0b10000, 0b10011, 0b10001, 0b10001, 0b01111],
        b'H' => [0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001],
        b'I' => [0b01110, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
        b'J' => [0b00111, 0b00010, 0b00010, 0b00010, 0b00010, 0b10010, 0b01100],
        b'K' => [0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001],
        b'L' => [0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111],
        b'M' => [0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001],
        b'N' => [0b10001, 0b11001, 0b11001, 0b10101, 0b10011, 0b10011, 0b10001],
        b'O' => [0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        b'P' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000],
        b'Q' => [0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101],
        b'R' => [0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001],
        b'S' => [0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110],
        b'T' => [0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100],
        b'U' => [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110],
        b'V' => [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100],
        b'W' => [0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b11011, 0b10001],
        b'X' => [0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001],
        b'Y' => [0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100],
        b'Z' => [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111],
        b'.' => [0, 0, 0, 0, 0, 0b01100, 0b01100],
        b'/' => [0b00001, 0b00010, 0b00010, 0b00100, 0b01000, 0b01000, 0b10000],
        b'-' => [0, 0, 0, 0b11111, 0, 0, 0],
        b'|' => [0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100],
        b'%' => [0b11001, 0b11010, 0b00010, 0b00100, 0b01000, 0b01011, 0b10011],
        b':' => [0, 0b00100, 0b00100, 0, 0b00100, 0b00100, 0],
        b' ' => [0; 7],
        _ => [0b11111, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b11111],
    }
}

/// The box's size in pixels.
const fn size(scale: u32) -> (u32, u32) {
    (BASE_W * scale, BASE_H * scale)
}

pub struct Canvas {
    pub w: usize,
    pub h: usize,
    pub rgb: Vec<[u8; 3]>,
}

impl Canvas {
    fn new(w: u32, h: u32, fill: [u8; 3]) -> Canvas {
        Canvas { w: w as usize, h: h as usize, rgb: vec![fill; (w * h) as usize] }
    }

    fn rect(&mut self, x: u32, y: u32, w: u32, h: u32, color: [u8; 3]) {
        let x1 = (x + w).min(self.w as u32) as usize;
        let y1 = (y + h).min(self.h as u32) as usize;
        for yy in (y as usize).min(y1)..y1 {
            self.rgb[yy * self.w + (x as usize).min(x1)..yy * self.w + x1].fill(color);
        }
    }

    fn border(&mut self, x: u32, y: u32, w: u32, h: u32, color: [u8; 3]) {
        if w == 0 || h == 0 {
            return;
        }
        self.rect(x, y, w, 1, color);
        self.rect(x, y + h - 1, w, 1, color);
        self.rect(x, y, 1, h, color);
        self.rect(x + w - 1, y, 1, h, color);
    }

    /// Draws `text` at `(x, y)` with integer glyph pixel size `px` (`2` => 10x14).
    /// Returns the ending `x` coordinate.
    fn text(&mut self, mut x: u32, y: u32, px: u32, color: [u8; 3], text: &str) -> u32 {
        let step = (GLYPH.0 + 1) * px;
        for ch in text.bytes() {
            let bits = glyph(ch.to_ascii_uppercase());
            for (row, &mask) in bits.iter().enumerate() {
                if mask == 0 {
                    continue;
                }
                for col in 0..GLYPH.0 {
                    if (mask >> (GLYPH.0 - 1 - col)) & 1 != 0 {
                        self.rect(x + col * px, y + row as u32 * px, px, px, color);
                    }
                }
            }
            x += step;
        }
        x
    }

    /// Draws `text` with `|` separators rendered in `SEP`.
    fn text_sep(&mut self, mut x: u32, y: u32, px: u32, color: [u8; 3], text: &str) -> u32 {
        for (i, part) in text.split('|').enumerate() {
            if i > 0 {
                x = self.text(x, y, px, SEP, "|");
            }
            x = self.text(x, y, px, color, part);
        }
        x
    }

}

/// CPU brand name from CPUID (`0x80000002..=0x80000004`), trimmed.
pub fn cpu_name() -> &'static str {
    static NAME: OnceLock<String> = OnceLock::new();
    NAME.get_or_init(|| {
        #[cfg(target_arch = "x86_64")]
        {
            let mut raw = [0u8; 48];
            for (i, leaf) in (0x8000_0002u32..=0x8000_0004).enumerate() {
                let r = std::arch::x86_64::__cpuid(leaf);
                raw[i * 16..i * 16 + 4].copy_from_slice(&r.eax.to_le_bytes());
                raw[i * 16 + 4..i * 16 + 8].copy_from_slice(&r.ebx.to_le_bytes());
                raw[i * 16 + 8..i * 16 + 12].copy_from_slice(&r.ecx.to_le_bytes());
                raw[i * 16 + 12..i * 16 + 16].copy_from_slice(&r.edx.to_le_bytes());
            }
            let s = String::from_utf8_lossy(&raw);
            let clean = s
                .trim_matches(|c: char| c == '\0' || c.is_whitespace())
                // i95: the 5x7 font has no (R)/(TM) glyphs ("Intel(R) Core(TM)" drew as "INTELORO COREOTMO").
                .replace("(R)", "")
                .replace("(TM)", "")
                .replace("8-Core Processor", "")
                .replace("16-Core Processor", "")
                .replace("12-Core Processor", "")
                .replace("6-Core Processor", "")
                .replace("Processor", "");
            clean.split_whitespace().collect::<Vec<_>>().join(" ")
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            "CPU".to_owned()
        }
    })
}


/// System RAM: `(used_phys, total_phys, used_commit, total_commit)` in bytes.
pub fn system_memory() -> (u64, u64, u64, u64) {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    // SAFETY: plain sized query.
    let mut m: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    m.dwLength = size_of::<MEMORYSTATUSEX>() as u32;
    if unsafe { GlobalMemoryStatusEx(&mut m) } == 0 {
        return (0, 0, 0, 0);
    }
    (
        m.ullTotalPhys.saturating_sub(m.ullAvailPhys),
        m.ullTotalPhys,
        m.ullTotalPageFile.saturating_sub(m.ullAvailPageFile),
        m.ullTotalPageFile,
    )
}

/// Per-logical-CPU utilization tracker via `NtQuerySystemInformation(8)`.
#[derive(Default)]
pub struct CpuMeter {
    pub prev: Vec<(i64, i64)>,
    pub total_pct: f32,
    pub cores_pct: Vec<f32>,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct SysProcPerfInfo {
    idle: i64,
    kernel: i64,
    user: i64,
    dpc: i64,
    interrupt: i64,
    interrupt_count: u32,
    _pad: u32,
}

impl CpuMeter {
    pub fn sample(&mut self) {
        use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleA, GetProcAddress};
        type NtQuery = unsafe extern "system" fn(u32, *mut u8, u32, *mut u32) -> i32;
        static FN: OnceLock<Option<NtQuery>> = OnceLock::new();
        let query = *FN.get_or_init(|| unsafe {
            let ntdll = GetModuleHandleA(c"ntdll.dll".as_ptr().cast());
            if ntdll.is_null() {
                return None;
            }
            let p = GetProcAddress(ntdll, c"NtQuerySystemInformation".as_ptr().cast())?;
            Some(std::mem::transmute::<unsafe extern "system" fn() -> isize, NtQuery>(p))
        });
        let Some(query) = query else { return };
        let n = std::thread::available_parallelism().map_or(8, |n| n.get()).min(64);
        let mut buf = vec![SysProcPerfInfo::default(); n];
        let mut ret = 0u32;
        // SystemProcessorPerformanceInformation = 8.
        let status = unsafe {
            query(8, buf.as_mut_ptr().cast(), (buf.len() * size_of::<SysProcPerfInfo>()) as u32, &mut ret)
        };
        if status < 0 {
            return;
        }
        let count = (ret as usize / size_of::<SysProcPerfInfo>()).min(buf.len());
        if count == 0 {
            return;
        }
        let now: Vec<(i64, i64)> = buf[..count].iter().map(|p| (p.idle, p.kernel + p.user)).collect();
        if self.prev.len() == count {
            self.cores_pct.clear();
            let mut sum = 0.0f32;
            for (&(pi, pt), &(ni, nt)) in self.prev.iter().zip(&now) {
                let dt = (nt - pt).max(1);
                let di = (ni - pi).clamp(0, dt);
                let busy = ((1.0 - di as f32 / dt as f32) * 100.0).clamp(0.0, 100.0);
                self.cores_pct.push(busy);
                sum += busy;
            }
            self.total_pct = sum / count as f32;
        }
        self.prev = now;
    }
}

/// The counters read for it. They grow, and go back to 0 with the frame-rate
/// line (right behind a look of ours).

/// Snapshot of data rendered onto the HUD canvas.
pub struct Snapshot {
    pub fps: f64,
    pub frame_ms: f64,
    pub gpu_ms: f64,
    pub gpu_pct: u32,
    pub gpu_measured: bool,
    pub submit_ms: f64,
    pub low1_fps: f64,
    pub max_ms: f64,
    pub draws: u64,
    pub dropped: u64,
    pub width: u32,
    pub height: u32,
    pub gpu_name: String,
    pub vram_used_gb: f64,
    pub vram_budget_gb: f64,
    pub vram_shared_gb: f64,
    pub vram_rt_gb: f64,
    pub vram_tex_gb: f64,
    pub vram_buf_gb: f64,
    pub vram_bvh_gb: f64,
    pub vram_oth_gb: f64,
    pub cpu_name: String,
    pub cpu_pct: f32,
    pub cores_pct: Vec<f32>,
    pub proc_ram_gb: f64,
    pub sys_ram_used_gb: f64,
    pub sys_ram_total_gb: f64,
    pub sys_commit_gb: f64,
    pub history_ms: Vec<f32>,
    /// The runtime's name (Vice-Tay's "VICTORY" row).
    pub runtime: String,
    /// What the runtime's log says a frame costs (i95: in place of the VRAM split, which only the runtime knows).
    pub detail: String,
}


fn fps_color(fps: f64) -> [u8; 3] {
    if fps >= 25.0 {
        GREEN
    } else if fps >= 14.0 {
        YELLOW
    } else {
        ORANGE
    }
}

fn frametime_color(ms: f32) -> [u8; 3] {
    if ms <= 38.0 {
        GREEN
    } else if ms <= 72.0 {
        YELLOW
    } else if ms <= 115.0 {
        ORANGE
    } else {
        RED
    }
}

fn trunc(s: &str, max: usize) -> String {
    if s.len() <= max { s.to_owned() } else { s[..max].to_owned() }
}

/// Paints the full colour HUD canvas.
pub fn paint_hud(s: &Snapshot) -> Canvas {
    let mut c = Canvas::new(BASE_W, BASE_H, BG);
    c.border(0, 0, BASE_W, BASE_H, BORDER);

    // 1. Large coloured FPS header.
    let fps_str = format!("{:.0} FPS", s.fps.round());
    c.text(14, 12, 4, fps_color(s.fps), &fps_str);
    // Subtle FPS decimal & ms badge on the right of the header.
    let badge = format!("{:.1} FPS", s.fps);
    c.text(BASE_W - 14 - badge.len() as u32 * 12, 24, 2, MUTED, &badge);

    // 2. Resolution | draws | GPU ms.
    let row2 = format!("{}X{} | {} DRAWS | GPU EST {:.1}MS", s.width, s.height, s.draws, s.gpu_ms);
    c.text_sep(14, 48, 2, WHITE, &trunc(&row2, 43));

    // 3. Frame | submit | 1% low | max.
    let row3 = format!(
        "FRAME {:.1} | SUB {:.1} | 1% {:.0} | MAX {:.0}",
        s.frame_ms, s.submit_ms, s.low1_fps, s.max_ms
    );
    c.text_sep(14, 68, 2, MUTED, &trunc(&row3, 43));

    // 4. Live frame-time sparkline graph.
    let (gx, gy, gw, gh) = (12u32, 90u32, BASE_W - 24, 76u32);
    c.rect(gx, gy, gw, gh, GRAPH_BG);
    c.border(gx, gy, gw, gh, BORDER);

    // Scale graph ceiling dynamically (min 100 ms, up to 200 ms if spikes exist).
    let peak = s.history_ms.iter().copied().fold(100.0f32, f32::max).min(220.0);
    let max_scale = if peak > 140.0 { 200.0 } else if peak > 90.0 { 133.0 } else { 100.0 };

    // Reference lines at 33 ms (30 FPS) and 66 ms (15 FPS).
    for &ref_ms in &[33.3f32, 66.7f32] {
        if ref_ms < max_scale {
            let ly = gy + gh - 2 - ((ref_ms / max_scale) * (gh - 4) as f32) as u32;
            c.rect(gx + 1, ly, gw - 2, 1, GRAPH_GRID);
            c.text(gx + 4, ly.saturating_sub(8), 1, MUTED, &format!("{:.0}", ref_ms));
        }
    }

    // Bars (3 px wide each, right-aligned so newest frame is on the right).
    let bar_w = 3u32;
    let slots = (gw - 4) / bar_w;
    let n = s.history_ms.len().min(slots as usize);
    let start_x = gx + gw - 2 - (n as u32) * bar_w;
    for (i, &ms) in s.history_ms[s.history_ms.len() - n..].iter().enumerate() {
        let frac = (ms / max_scale).clamp(0.02, 1.0);
        let h = ((frac * (gh - 4) as f32) as u32).max(2);
        let bx = start_x + i as u32 * bar_w;
        let by = gy + gh - 2 - h;
        let top_col = frametime_color(ms);
        // Warm base fill + crisp coloured cap/spike (matches reference style).
        let cap = h.min(if ms > 72.0 { h } else { 5 });
        if h > cap {
            c.rect(bx, by + cap, bar_w - 1, h - cap, GRAPH_FLOOR);
        }
        c.rect(bx, by, bar_w - 1, cap, top_col);
    }

    // 5. Hardware & memory rows.
    let val_x = 104u32;
    let max_chars = ((BASE_W - val_x - 10) / 12) as usize;

    // GPU
    c.text(14, 176, 2, LABEL, "GPU");
    let source = if s.gpu_measured { "APP" } else { "EST" };
    let load = format!("{source} {}%", s.gpu_pct);
    let gpu_line = format!("{} {load}", trunc(&s.gpu_name, max_chars.saturating_sub(load.len() + 1)));
    c.text(val_x, 176, 2, WHITE, &trunc(&gpu_line, max_chars));

    // VRAM
    c.text(14, 196, 2, LABEL, "VRAM");
    let vram_col = if s.vram_budget_gb > 0.0 && s.vram_used_gb / s.vram_budget_gb >= 0.92 {
        ORANGE
    } else {
        WHITE
    };
    let vram_line = format!(
        "{:.1}/{:.1} GB | SHR {:.1} GB",
        s.vram_used_gb, s.vram_budget_gb, s.vram_shared_gb
    );
    c.text_sep(val_x, 196, 2, vram_col, &trunc(&vram_line, max_chars));

    // i95: the log's cost of a frame, in place of Vice-Tay's VRAM split.
    c.text_sep(14, 216, 2, MUTED, &trunc(&s.detail, 43));

    // CPU + per-core bars
    c.text(14, 236, 2, LABEL, "CPU");
    let cpu_line = format!("{} {:.0}%", trunc(&s.cpu_name, max_chars.saturating_sub(5)), s.cpu_pct);
    c.text(val_x, 236, 2, WHITE, &trunc(&cpu_line, max_chars));

    let cores = s.cores_pct.len().clamp(1, 32);
    let area_w = BASE_W - val_x - 14;
    let gap = 2u32;
    let cw = ((area_w + gap) / cores as u32).saturating_sub(gap).max(4);
    let bar_h = 22u32;
    let bar_y = 256u32;
    for (i, &pct) in s.cores_pct.iter().take(cores).enumerate() {
        let bx = val_x + i as u32 * (cw + gap);
        c.rect(bx, bar_y, cw, bar_h, BAR_BG);
        let fh = ((pct / 100.0) * bar_h as f32).round() as u32;
        if fh > 0 {
            let fh = fh.min(bar_h);
            let col = if pct >= 85.0 { BAR_FILL_HI } else { BAR_FILL_LO };
            c.rect(bx, bar_y + bar_h - fh, cw, fh, col);
        }
    }

    // VICTORY
    c.text(14, 288, 2, LABEL, &trunc(&s.runtime.to_uppercase(), 7));
    let vic_line = format!("RAM {:.1} GB | DROP {}", s.proc_ram_gb, s.dropped);
    c.text_sep(val_x, 288, 2, WHITE, &trunc(&vic_line, max_chars));

    // RAM
    c.text(14, 308, 2, LABEL, "RAM");
    let ram_line = format!(
        "{:.1}/{:.1} GB | CMT {:.1}",
        s.sys_ram_used_gb, s.sys_ram_total_gb, s.sys_commit_gb
    );
    c.text_sep(val_x, 308, 2, WHITE, &trunc(&ram_line, max_chars));

    c
}


// ---- i95: what Vice-Tay reads inside its renderer, read from outside it ----

/// The first adapter's name and dedicated memory, from DXGI (once).
pub fn gpu_info() -> &'static (String, u64) {
    static INFO: OnceLock<(String, u64)> = OnceLock::new();
    INFO.get_or_init(|| {
        use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1};
        let found = (|| unsafe {
            let factory: IDXGIFactory1 = CreateDXGIFactory1().ok()?;
            let desc = factory.EnumAdapters1(0).ok()?.GetDesc1().ok()?;
            let end = desc.Description.iter().position(|&c| c == 0).unwrap_or(desc.Description.len());
            Some((String::from_utf16_lossy(&desc.Description[..end]).trim().to_owned(), desc.DedicatedVideoMemory as u64))
        })();
        found.unwrap_or_else(|| ("GPU".to_owned(), 0))
    })
}

/// The runtime process's committed private bytes.
pub fn process_private_bytes(pid: u32) -> u64 {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return 0;
        }
        let mut c = PROCESS_MEMORY_COUNTERS { cb: size_of::<PROCESS_MEMORY_COUNTERS>() as u32, ..Default::default() };
        K32GetProcessMemoryInfo(h, &mut c, c.cb);
        CloseHandle(h);
        c.PagefileUsage as u64
    }
}

/// The runtime process's GPU load and memory from Windows' performance
/// counters (what Task Manager shows): 3D engine %, dedicated and shared bytes.
pub struct GpuWatch {
    pid: u32,
    query: isize,
    engine: isize,
    dedicated: isize,
    shared: isize,
}

impl GpuWatch {
    pub fn new(pid: u32) -> Option<GpuWatch> {
        use windows_sys::Win32::System::Performance::*;
        unsafe {
            let mut q = 0isize;
            if PdhOpenQueryW(std::ptr::null(), 0, &mut q as *mut isize as _) != 0 {
                return None;
            }
            let add = |path: String| {
                let w: Vec<u16> = path.encode_utf16().chain([0]).collect();
                let mut c = 0isize;
                (PdhAddEnglishCounterW(q as _, w.as_ptr(), 0, &mut c as *mut isize as _) == 0).then_some(c).unwrap_or(0)
            };
            let engine = add(format!("\\GPU Engine(pid_{pid}_*engtype_3D)\\Utilization Percentage"));
            let dedicated = add(format!("\\GPU Process Memory(pid_{pid}_*)\\Dedicated Usage"));
            let shared = add(format!("\\GPU Process Memory(pid_{pid}_*)\\Shared Usage"));
            PdhCollectQueryData(q as _);
            Some(GpuWatch { pid, query: q, engine, dedicated, shared })
        }
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// (3D load %, dedicated bytes, shared bytes) since the last call.
    pub fn sample(&self) -> (f64, f64, f64) {
        use windows_sys::Win32::System::Performance::*;
        let sum = |c: isize| -> f64 {
            if c == 0 {
                return 0.0;
            }
            unsafe {
                let (mut size, mut count) = (0u32, 0u32);
                PdhGetFormattedCounterArrayW(c as _, PDH_FMT_DOUBLE, &mut size, &mut count, std::ptr::null_mut());
                if size == 0 {
                    return 0.0;
                }
                let mut buf = vec![0u8; size as usize];
                let items = buf.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W;
                if PdhGetFormattedCounterArrayW(c as _, PDH_FMT_DOUBLE, &mut size, &mut count, items) != 0 {
                    return 0.0;
                }
                (0..count as usize).map(|i| (*items.add(i)).FmtValue.Anonymous.doubleValue).filter(|v| v.is_finite()).sum()
            }
        };
        unsafe { PdhCollectQueryData(self.query as _) };
        (sum(self.engine).min(100.0), sum(self.dedicated), sum(self.shared))
    }
}

impl Drop for GpuWatch {
    fn drop(&mut self) {
        unsafe { windows_sys::Win32::System::Performance::PdhCloseQuery(self.query as _) };
    }
}
