//! The title's frame rate (`VICTORY_FRAME_INTERVAL_MS`, off without it): the
//! rate the title asks for is a setting of its own, and places in its code are
//! changed at load to read another.
//!
//! What the title does:
//!
//! - Its graphics settings hold a frame rate (30 on Series X). Two functions
//!   that apply the settings turn it into a frame time: `frame_us = 1e6 /
//!   clamp(rate)`, 0 and 1000 or more being 1000 (the code's "no limit").
//! - A function sets the frame interval from it: the display's rate is 60, or
//!   120 if its mode list (the output's `GetDisplayModeList`) has a 120 Hz
//!   mode; the divisor is `min(display / rate, 4)`, or 1 for a rate above the
//!   display's; the interval comes from a table (`[16667, 16667, 33333, 50000,
//!   66667]` at 60 Hz, `[8333, 8333, 16667, 25000, 33333]` at 120 Hz) and goes
//!   to `SetFrameIntervalX`, with `ScheduleFrameEventX` after it. At 30 and 60
//!   Hz: 33,333 us, the flat 33.3 ms of the menus and videos.
//! - Once a flag is up, a limiter of its own on `QueryPerformanceCounter` waits
//!   until `1000 / trunc(1e6 / frame_us)` ms have passed since the last frame.
//! - What scripts are told the rate by, and what scales per-frame amounts by it,
//!   follow the same frame time.
//! - The simulation: each of its timers is handed the real time since the last
//!   frame (`QueryPerformanceCounter`); the step of it is at most 1/15 s and at
//!   least `1 / rate`, which a function reads from the same setting. So at 30
//!   every frame shorter than 33.3 ms still moves game time 33.3 ms: a fixed 30
//!   Hz step for fast frames, delta time for slow ones.
//!
//! So reporting a 120 Hz display alone changes nothing at 30 (divisor 4,
//! 33,333 us). Here the two loads of the setting that make the frame time
//! become `mov eax, rate` and nops, so the title's frame time, interval and own
//! limiter follow the rate asked for, and the check the shortest step is made
//! of returns the rate too (1000 for no limit); the setting itself is left as
//! it is.
//!
//! Where those places are is not in this repository: the user's profile names
//! them as byte patterns (`frame.read`, `frame.step` of [`crate::title_profile`]),
//! each of which must match exactly once. A pattern that matches nowhere or twice
//! (another build of the title) leaves its place alone and says so; without a
//! profile the title's own rate stays and only our frame clock ticks at the
//! interval asked for.

use crate::title_profile::{Code, Profile, find_once, note};

/// The switch: the interval in milliseconds our frame clock ticks at
/// (`victory-gpu`'s `frame`), 0 for no wait at all. `None` if unset or
/// malformed (said why).
pub fn asked_interval_ms() -> Option<f64> {
    let spec = std::env::var("VICTORY_FRAME_INTERVAL_MS").ok()?;
    let ms = parse_interval_ms(&spec);
    if ms.is_none() {
        eprintln!("VIctory: VICTORY_FRAME_INTERVAL_MS={spec}: expected milliseconds from 0 (no wait) to 1000");
    }
    ms
}

/// `16.667` gives 16.667; 0 is allowed (no wait), more than a second is not.
pub fn parse_interval_ms(spec: &str) -> Option<f64> {
    spec.trim().parse::<f64>().ok().filter(|ms| ms.is_finite() && (0.0..=1000.0).contains(ms))
}

/// The frame rate the title is given for an interval: `1000 / ms` rounded
/// (16.667 gives 60, 8.333 gives 120, 33.333 gives 30), 0 for no wait (the
/// title's own "no limit"). `VICTORY_TITLE_FPS=<rate>` gives it outright, `off`
/// leaves the title as it is (only our clock changes).
pub fn title_rate(interval_ms: f64, title_fps: Option<&str>) -> Option<u32> {
    match title_fps.map(str::trim) {
        Some("off") => None,
        Some(rate) => rate.parse::<u32>().ok().filter(|r| *r <= 1000),
        None if interval_ms <= 0.0 => Some(0),
        None => Some(((1000.0 / interval_ms).round() as u32).clamp(1, 1000)),
    }
}

/// The frame time the title derives from a rate, in microseconds, as its
/// code computes it (single-precision division, truncated).
pub fn title_frame_us(rate: u32) -> u32 {
    let clamped = if rate == 0 || rate >= 1000 { 1000 } else { rate };
    (1_000_000.0_f32 / clamped as f32) as u32
}

/// The interval the title then asks for (`SetFrameIntervalX`), in
/// microseconds, on a display of 60 Hz (or 120 Hz).
pub fn title_interval_us(rate: u32, display_120: bool) -> u32 {
    const TABLE: [[u32; 5]; 2] = [[16667, 16667, 33333, 50000, 66667], [8333, 8333, 16667, 25000, 33333]];
    let fps = (1_000_000.0_f32 / title_frame_us(rate) as f32) as u32 & 0xffff;
    let display = if display_120 { 120 } else { 60 };
    let index = if fps <= display { (display / fps.max(1)).min(4) } else { 1 };
    TABLE[usize::from(display_120)][index as usize]
}

/// What is changed at a place of the title's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A load of the rate setting before the frame time is made of it:
    /// `len` bytes become `mov eax, rate` and nops.
    Load { len: usize },
    /// The check the timers' shortest step is made of (`1 / rate`): 7 bytes
    /// become `mov eax, rate ; jmp` `jump` bytes on from their end, to the
    /// conversion of the rate. Left as it was, the timers step at least 1/30 s
    /// however short the frame: at 60 frames a second the game ran at twice its
    /// speed (run fps-1, menus). For no limit, 1000.
    SimRate { jump: u8 },
}

/// A place of the title's to change: its offset in the image and what is
/// changed there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Site {
    pub at: usize,
    pub kind: Kind,
}

/// Each place of the profile found once in `code`, and a note for each that
/// was not.
pub fn locate(profile: &Profile, code: &Code) -> (Vec<Site>, Vec<String>) {
    let (mut sites, mut notes) = (Vec::new(), Vec::new());
    for (n, place) in profile.frame_reads.iter().enumerate() {
        match find_once(code, &place.pattern) {
            Ok(found) => sites.push(Site { at: found + place.at, kind: Kind::Load { len: place.len } }),
            Err(why) => notes.push(note("frame.read", n, &why)),
        }
    }
    for (n, step) in profile.frame_steps.iter().enumerate() {
        match find_once(code, &step.pattern) {
            Ok(found) => sites.push(Site { at: found + step.at, kind: Kind::SimRate { jump: step.jump } }),
            Err(why) => notes.push(note("frame.step", n, &why)),
        }
    }
    (sites, notes)
}

/// What goes at a site of `kind` for `rate`.
pub fn replacement(kind: Kind, rate: u32) -> Vec<u8> {
    match kind {
        // mov eax, rate ; nop...
        Kind::Load { len } => {
            let r = rate.to_le_bytes();
            let mut code = vec![0xb8, r[0], r[1], r[2], r[3]];
            code.resize(len.max(5), 0x90);
            code
        }
        // mov eax, rate ; jmp +jump
        Kind::SimRate { jump } => {
            let r = (if rate == 0 { 1000 } else { rate }).to_le_bytes();
            vec![0xb8, r[0], r[1], r[2], r[3], 0xeb, jump]
        }
    }
}

/// What became of a site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Patched {
    Done,
    /// Outside the image, or Windows refused.
    Failed(&'static str),
}

/// Makes each site of `sites` in the image of `size` bytes at `base` load
/// `rate`. What became of each, said in the log too.
///
/// # Safety
/// The image stays mapped and no thread runs its code yet.
#[cfg(windows)]
pub unsafe fn apply(base: usize, size: usize, rate: u32, sites: &[Site]) -> Vec<Patched> {
    use windows_sys::Win32::System::Diagnostics::Debug::FlushInstructionCache;
    use windows_sys::Win32::System::Memory::{PAGE_EXECUTE_READWRITE, VirtualProtect};
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    sites
        .iter()
        .map(|site| {
            let code = replacement(site.kind, rate);
            let len = code.len();
            let done = (|| {
                if site.at.checked_add(len).is_none_or(|end| end > size) {
                    return Patched::Failed("outside the image");
                }
                let at = base + site.at;
                let mut old = 0;
                // SAFETY: inside the mapped image (checked above); the bytes are
                // made writable for this and put back; nothing runs them yet.
                unsafe {
                    if VirtualProtect(at as *const _, len, PAGE_EXECUTE_READWRITE, &mut old) == 0 {
                        return Patched::Failed("Windows would not make it writable");
                    }
                    std::ptr::copy_nonoverlapping(code.as_ptr(), at as *mut u8, len);
                    VirtualProtect(at as *const _, len, old, &mut old);
                    FlushInstructionCache(GetCurrentProcess(), at as *const _, len);
                }
                Patched::Done
            })();
            let offset = format_args!("{:#x}", site.at);
            match done {
                Patched::Done => tracing::info!(site = %offset, kind = ?site.kind, rate, "frame rate: the title's read of its frame rate setting patched (VICTORY_FRAME_INTERVAL_MS)"),
                Patched::Failed(why) => tracing::warn!(site = %offset, why, "frame rate: not patched"),
            }
            done
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intervals_and_rates() {
        assert_eq!(parse_interval_ms("16.667"), Some(16.667));
        assert_eq!(parse_interval_ms(" 0 "), Some(0.0));
        assert_eq!(parse_interval_ms("-1"), None);
        assert_eq!(parse_interval_ms("fast"), None);
        assert_eq!(parse_interval_ms("1001"), None);
        assert_eq!(title_rate(16.667, None), Some(60));
        assert_eq!(title_rate(8.333, None), Some(120));
        assert_eq!(title_rate(33.333, None), Some(30));
        assert_eq!(title_rate(0.0, None), Some(0));
        assert_eq!(title_rate(0.5, None), Some(1000));
        assert_eq!(title_rate(16.667, Some("off")), None);
        assert_eq!(title_rate(0.0, Some("90")), Some(90));
        assert_eq!(title_rate(0.0, Some("lots")), None);
    }

    /// The title's own arithmetic: its 30 is the 33,333 us the log shows,
    /// on a 60 Hz display and on a 120 Hz one.
    #[test]
    fn the_titles_intervals() {
        assert_eq!(title_frame_us(30), 33333);
        assert_eq!(title_frame_us(60), 16666);
        assert_eq!(title_frame_us(0), 1000);
        assert_eq!(title_frame_us(5000), 1000);
        assert_eq!(title_interval_us(30, false), 33333);
        assert_eq!(title_interval_us(30, true), 33333);
        assert_eq!(title_interval_us(40, true), 25000);
        assert_eq!(title_interval_us(60, false), 16667);
        assert_eq!(title_interval_us(60, true), 16667);
        assert_eq!(title_interval_us(120, true), 8333);
        assert_eq!(title_interval_us(120, false), 16667);
        assert_eq!(title_interval_us(0, false), 16667);
        assert_eq!(title_interval_us(0, true), 8333);
        assert_eq!(title_interval_us(20, false), 50000);
    }

    #[test]
    fn what_goes_over_a_site() {
        assert_eq!(replacement(Kind::Load { len: 6 }, 60), [0xb8, 60, 0, 0, 0, 0x90]);
        assert_eq!(replacement(Kind::Load { len: 8 }, 120), [0xb8, 120, 0, 0, 0, 0x90, 0x90, 0x90]);
        assert_eq!(replacement(Kind::Load { len: 5 }, 1), [0xb8, 1, 0, 0, 0]);
        assert_eq!(replacement(Kind::SimRate { jump: 11 }, 60), [0xb8, 60, 0, 0, 0, 0xeb, 11]);
        assert_eq!(replacement(Kind::SimRate { jump: 11 }, 0)[1..5], 1000u32.to_le_bytes());
    }

    #[test]
    fn the_profiles_places_are_found_once_each() {
        let profile = Profile::parse(
            "frame.read = 2 6 : 11 22 AA BB CC DD EE FF 33\n\
             frame.read = 0 6 : 99 98 97 96 95 94\n\
             frame.step = 1 11 : 77 01 02 03 04 05 06 07 78\n",
        )
        .unwrap();
        let mut code = vec![0x90u8; 0x10];
        code.extend([0x11, 0x22, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff, 0x33]);
        code.extend([0x90; 4]);
        let step_at = code.len();
        code.extend([0x77, 1, 2, 3, 4, 5, 6, 7, 0x78]);
        let (sites, notes) = locate(&profile, &[(0x2000, &code)]);
        assert_eq!(
            sites,
            [
                Site { at: 0x2000 + 0x10 + 2, kind: Kind::Load { len: 6 } },
                Site { at: 0x2000 + step_at + 1, kind: Kind::SimRate { jump: 11 } },
            ]
        );
        assert_eq!(notes.len(), 1);
        assert!(notes[0].starts_with("frame.read 2: not patched") && notes[0].contains("not in this build"), "{notes:?}");
    }

    #[cfg(windows)]
    #[test]
    fn sites_take_the_rate_and_others_are_left_alone() {
        use windows_sys::Win32::System::Memory::{MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_EXECUTE_READWRITE, VirtualAlloc, VirtualFree};
        // SAFETY: fresh memory of our own, written and run inside it.
        unsafe {
            let place = VirtualAlloc(std::ptr::null(), 0x10000, MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE) as *mut u8;
            assert!(!place.is_null());
            // The settings: 30 at +0x24.
            let mut settings = [0u32; 0x20];
            settings[9] = 30;
            let base = (settings.as_ptr() as u64).to_le_bytes();
            // fn() -> u32: the load of the setting.
            let mut load = vec![0x48, 0xb8];
            load.extend(base); // mov rax, settings
            let load_at = load.len();
            load.extend([0x8b, 0x80, 0x24, 0, 0, 0]); // mov eax, [rax+24h]
            load.push(0xc3);
            std::ptr::copy_nonoverlapping(load.as_ptr(), place.add(0x100), load.len());
            // fn() -> f32: the check of the setting and the conversion of the rate.
            let mut step = vec![0x48, 0xb8];
            step.extend(base);
            let step_at = step.len();
            step.extend([0x83, 0xb8, 0x24, 0, 0, 0, 0]); // cmp dword [rax+24h], 0
            step.extend([0x74, 0x03, 0x0f, 0x1f, 0x00]); // je +3 ; nop dword [rax]
            step.extend([0x8b, 0x80, 0x24, 0, 0, 0]); // mov eax, [rax+24h]
            step.extend([0xc4, 0xe1, 0xfa, 0x2a, 0xc0, 0xc3]); // vcvtsi2ss xmm0, xmm0, rax ; ret
            std::ptr::copy_nonoverlapping(step.as_ptr(), place.add(0x200), step.len());
            let read = std::mem::transmute::<*mut u8, extern "system" fn() -> u32>(place.add(0x100));
            let rate = std::mem::transmute::<*mut u8, extern "system" fn() -> f32>(place.add(0x200));
            assert_eq!((read(), rate()), (30, 30.0), "as the title has it");
            // Three places: the load, the check (jumping 11 bytes on from its 7, to the
            // conversion), and one that is not in the image.
            let sites = [
                Site { at: 0x100 + load_at, kind: Kind::Load { len: 6 } },
                Site { at: 0x200 + step_at, kind: Kind::SimRate { jump: 11 } },
                Site { at: 0xfffe, kind: Kind::Load { len: 6 } },
            ];
            let done = apply(place as usize, 0x10000, 60, &sites);
            assert_eq!(done[..2], [Patched::Done, Patched::Done]);
            assert!(matches!(done[2], Patched::Failed(_)));
            assert_eq!(std::slice::from_raw_parts(place.add(0x100 + load_at), 6), replacement(Kind::Load { len: 6 }, 60));
            assert_eq!((read(), rate()), (60, 60.0));
            // No limit is a rate of 1000 for the step.
            apply(place as usize, 0x10000, 0, &sites[1..2]);
            assert_eq!(rate(), 1000.0);
            VirtualFree(place.cast(), 0, MEM_RELEASE);
            assert_eq!(settings[9], 30, "the setting itself is left as it is");
        }
    }
}
