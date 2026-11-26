//! VICTORY_TITLE_PROBE=1 (diagnostics): reads title globals and the frame lead-in once a second; never writes title memory

use std::sync::atomic::{AtomicBool, Ordering};

// title rvas (game_scarlett_master.exe 1.0.2.1)
const RENDER_THREAD: usize = 0x78aef40; // render thread object, its work semaphore at +0x1b38
const RENDER_SEMAPHORE: usize = 0x1b38;
const MAIN_SYNC: usize = 0xa210d90; // u32 waited, then qpc at the sync's start (+8) and return (+0x10)
const LOD_ROOT: usize = 0xa15e100;
const LOD_SETTING: usize = 0xa15e200;
const LOD_FLAGS: usize = 0xa15e238;
const CASCADES: usize = 0x84ddbe0;
const SPLITS: usize = 0x84ddb40;
const SHADOW_FLAGS: usize = 0x84dda00;

pub(crate) fn on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_TITLE_PROBE").is_some_and(|v| v == "1"))
}

/// Starts the probe thread once, if it is on.
pub(crate) fn start() {
    static STARTED: AtomicBool = AtomicBool::new(false);
    if !on() || STARTED.swap(true, Ordering::Relaxed) {
        return;
    }
    let Some(base) = title_base() else {
        tracing::warn!("title probe: the title's image was not found");
        return;
    };
    let _ = std::thread::Builder::new()
        .name("victory-title-probe".into())
        .spawn(move || run(base));
}

fn read<T: Copy + Default>(at: usize) -> Option<T> {
    let mut v = T::default();
    // SAFETY: a plain value's bytes, filled by a fault-safe read
    let bytes = unsafe { std::slice::from_raw_parts_mut((&raw mut v).cast::<u8>(), size_of::<T>()) };
    crate::host::read_into(at as *const u8, bytes).then_some(v)
}

fn qpc() -> i64 {
    let mut v = 0;
    // SAFETY: an out pointer to a local
    unsafe { windows_sys::Win32::System::Performance::QueryPerformanceCounter(&mut v) };
    v
}

fn qpc_ms(ticks: i64) -> f64 {
    static FREQ: std::sync::OnceLock<i64> = std::sync::OnceLock::new();
    let freq = *FREQ.get_or_init(|| {
        let mut f = 0;
        // SAFETY: an out pointer to a local
        unsafe { windows_sys::Win32::System::Performance::QueryPerformanceFrequency(&mut f) };
        f.max(1)
    });
    ticks as f64 * 1000.0 / freq as f64
}

// the largest image mapped from a victory-image-* file: the title itself
fn title_base() -> Option<usize> {
    use windows_sys::Win32::System::LibraryLoader::GetModuleFileNameW;
    use windows_sys::Win32::System::ProcessStatus::{K32EnumProcessModules, K32GetModuleInformation, MODULEINFO};
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    // SAFETY: queries of our own process into sized buffers
    unsafe {
        let me = GetCurrentProcess();
        let mut handles = vec![std::ptr::null_mut(); 2048];
        let mut bytes = 0;
        let size = (handles.len() * size_of::<windows_sys::Win32::Foundation::HMODULE>()) as u32;
        if K32EnumProcessModules(me, handles.as_mut_ptr(), size, &mut bytes) == 0 {
            return None;
        }
        handles.truncate(bytes as usize / size_of::<windows_sys::Win32::Foundation::HMODULE>());
        let mut best = None;
        for handle in handles {
            let mut info = MODULEINFO { lpBaseOfDll: std::ptr::null_mut(), SizeOfImage: 0, EntryPoint: std::ptr::null_mut() };
            if K32GetModuleInformation(me, handle, &mut info, size_of::<MODULEINFO>() as u32) == 0 {
                continue;
            }
            let mut path = [0u16; 520];
            let len = GetModuleFileNameW(handle, path.as_mut_ptr(), path.len() as u32) as usize;
            let path = String::from_utf16_lossy(&path[..len]).to_ascii_lowercase();
            let file = path.rsplit(['\\', '/']).next().unwrap_or(&path);
            if file.starts_with("victory-image-") && best.is_none_or(|(_, s)| info.SizeOfImage > s) {
                best = Some((handle as usize, info.SizeOfImage));
            }
        }
        best.map(|(b, _)| b)
    }
}

#[derive(Default)]
struct Lead {
    frames: u32,
    main: (f64, u32),
    woke: (f64, u32),
    waited: f64,
}

// per frame: the main thread's sync return and the render thread's wake, after our present
#[derive(Default)]
struct Frame {
    at: i64,
    main_seen: bool,
    woke_seen: bool,
}

fn run(base: usize) {
    let tick = std::time::Duration::from_micros(250);
    let mut lead = Lead::default();
    let mut frame = Frame::default();
    let mut presents = crate::host::presents();
    let mut last_word = 0u64;
    let mut logged = std::time::Instant::now();
    loop {
        std::thread::sleep(tick);
        let now = qpc();
        let p = crate::host::presents();
        if p != presents {
            presents = p;
            if frame.at != 0 {
                lead.frames += 1;
            }
            frame = Frame { at: now, ..Frame::default() };
        }
        if frame.at != 0 && !frame.main_seen
            && let Some([waited, _, _, _, back_lo, back_hi]) = read::<[u32; 6]>(base + MAIN_SYNC)
        {
            let back = i64::from(back_lo) | i64::from(back_hi) << 32;
            if back > frame.at && back <= now {
                frame.main_seen = true;
                lead.main.0 += qpc_ms(back - frame.at);
                lead.main.1 += 1;
                lead.waited += f64::from(waited);
            }
        }
        // the render thread waits on its semaphore after our present; its waiters going to none is its wake
        if let Some(object) = read::<usize>(base + RENDER_THREAD).filter(|&o| o != 0)
            && let Some(word) = read::<u64>(object + RENDER_SEMAPHORE)
        {
            let woke = (last_word & 0x7f != 0 && word & 0x7f == 0) || word >> 32 > last_word >> 32;
            if frame.at != 0 && !frame.woke_seen && woke {
                frame.woke_seen = true;
                lead.woke.0 += qpc_ms(now - frame.at);
                lead.woke.1 += 1;
            }
            last_word = word;
        }
        if logged.elapsed().as_secs() >= 1 {
            logged = std::time::Instant::now();
            report(base, &lead);
            lead = Lead::default();
        }
    }
}

fn report(base: usize, lead: &Lead) {
    let avg = |(sum, n): (f64, u32)| if n == 0 { f64::NAN } else { sum / f64::from(n) };
    let f = |rva: usize| read::<f32>(base + rva).unwrap_or(f32::NAN);
    let splits: Vec<f32> = (0..5).map(|k| f(SPLITS + 4 * k)).collect();
    tracing::info!(
        frames = lead.frames,
        main_back_ms = format_args!("{:.1}", avg(lead.main)),
        main_frames = lead.main.1,
        render_woke_ms = format_args!("{:.1}", avg(lead.woke)),
        woke_frames = lead.woke.1,
        main_waited_title_units = format_args!("{:.1}", if lead.main.1 == 0 { f64::NAN } else { lead.waited / f64::from(lead.main.1) }),
        lod_root = f(LOD_ROOT),
        lod_setting = f(LOD_SETTING),
        lod_flags = format_args!("{:#x}", read::<u8>(base + LOD_FLAGS).unwrap_or(0)),
        cascades = read::<i32>(base + CASCADES).unwrap_or(-1),
        splits = ?splits,
        shadow_flags = format_args!("{:#x}", read::<u32>(base + SHADOW_FLAGS).unwrap_or(0)),
        "title probe: after our present, ms until the main thread's sync returned (its own clock) and the render thread woke (polled, 0.25 ms); title globals (VICTORY_TITLE_PROBE=1)"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_are_fault_safe_and_exact() {
        let v: [u32; 6] = [7, 0, 1, 2, 3, 4];
        assert_eq!(read::<[u32; 6]>(v.as_ptr() as usize), Some(v));
        assert_eq!(read::<u64>(8), None);
    }

    #[test]
    fn no_title_image_in_a_test_process() {
        assert_eq!(title_base(), None);
    }
}
