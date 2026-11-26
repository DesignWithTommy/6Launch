//! The title's output size and render scale (`VICTORY_OUTPUT`,
//! `VICTORY_RENDER_SCALE`, `VICTORY_FFX_SCALE`, `VICTORY_UPSCALER`; each off
//! without it): places in the title's code changed at load.
//!
//! What the title does:
//!
//! - **Output size.** Its settings (a file in its storage, 3840x2160 as a
//!   Series X wrote it) are read into the live settings, the defaults'
//!   1920x1080 overwritten. A function that applies them hands the size to the
//!   one writer of the screen size, and calls another when the height changes.
//!   The back buffers take that size. The display mode list it matches against
//!   is its own, not the DXGI output's, which is why `VICTORY_DISPLAY_MODE`
//!   changes nothing. Here the block that applies them becomes: write the size
//!   asked for into the settings, then the title's own else-path (screen size
//!   set, the other function if the height changed). Everything that reads the
//!   settings after it (the device parameters, the render size) sees the same
//!   size. The back buffers' pool (64 MB on Series X) is checked to be used up
//!   but for 2 MB (an `int3`); for another size that check is jumped over.
//! - **Render scale.** The upscaler the title picks (its own, except on Series
//!   S with a flag, AMD FidelityFX) gives the render size from the output
//!   size: its own multiplies by a scale in its object, set to 2/3 (so
//!   2560x1440 for 4K, 1280x720 for 1080p); `VICTORY_RENDER_SCALE` replaces
//!   that constant (Series X; run res-4 at 1.0). FidelityFX's multiplies by 1.0
//!   on Series X, 2/3 on Series S; `VICTORY_FFX_SCALE` points its two loads at
//!   a constant in its padding (as pull request #53 did). That one is not
//!   safe: at 0.5 on Series S (run res-6) FidelityFX's jitter phase count came
//!   back 0 and the title divided by it 49 s in.
//! - **Upscaler.** `VICTORY_UPSCALER=<kind>` makes the picker return `kind`
//!   (pull request #53): 6 the title's own, 7 FidelityFX.
//!
//! Where those places are, and the bytes that go there, are not in this
//! repository: the user's profile names them (`resolution.*` of
//! [`crate::title_profile`]), as byte patterns that must each match exactly once.
//! A pattern that matches nowhere or twice (another build of the title) leaves
//! its place alone and says so. Without a profile each switch says `not
//! working: no profile` and the title is run as it is.

use crate::frame_rate::Patched;
use crate::title_profile::{Code, Edit, Profile, Values, find_once, parse_size};

/// `VICTORY_OUTPUT=<w>x<h>`: one of the title's `modes` (the profile's list; any
/// plausible size if it gives none). `None` if unset or not one (said why).
pub fn asked_output(modes: &[(u32, u32)]) -> Option<(u32, u32)> {
    let spec = std::env::var("VICTORY_OUTPUT").ok()?;
    let size = parse_output(&spec, modes);
    if size.is_none() {
        if modes.is_empty() {
            eprintln!("VIctory: VICTORY_OUTPUT={spec}: expected <width>x<height>");
        } else {
            let list: Vec<String> = modes.iter().map(|(w, h)| format!("{w}x{h}")).collect();
            eprintln!("VIctory: VICTORY_OUTPUT={spec}: expected one of the title's modes: {}", list.join(", "));
        }
    }
    size
}

pub fn parse_output(spec: &str, modes: &[(u32, u32)]) -> Option<(u32, u32)> {
    let size = parse_size(spec)?;
    let plausible = (160..=16384).contains(&size.0) && (90..=16384).contains(&size.1);
    (if modes.is_empty() { plausible } else { modes.contains(&size) }).then_some(size)
}

/// `VICTORY_RENDER_SCALE=<s>`, 0.5 to 1.0 (0.5: half the output's width and
/// height): the title's own upscaler's scale (Series X; 2/3 without it).
/// Tested at 1.0 (run res-4); below 2/3 the title's largest render size stays
/// at 2/3. `None` if unset or out of range (said why).
pub fn asked_render_scale() -> Option<f32> {
    let spec = std::env::var("VICTORY_RENDER_SCALE").ok()?;
    let scale = parse_render_scale(&spec);
    if scale.is_none() {
        eprintln!("VIctory: VICTORY_RENDER_SCALE={spec}: expected 0.5 to 1.0");
    }
    scale
}

pub fn parse_render_scale(spec: &str) -> Option<f32> {
    spec.trim().parse::<f32>().ok().filter(|s| (0.5..=1.0).contains(s))
}

/// `VICTORY_FFX_SCALE=<s>`, 0.5 to 1.0: FidelityFX's render scale (Series
/// S). Crashed at 0.5 (run res-6); see the module's notes.
pub fn asked_ffx_scale() -> Option<f32> {
    let spec = std::env::var("VICTORY_FFX_SCALE").ok()?;
    let scale = parse_render_scale(&spec);
    if scale.is_none() {
        eprintln!("VIctory: VICTORY_FFX_SCALE={spec}: expected 0.5 to 1.0");
    }
    scale
}

/// `VICTORY_UPSCALER=<kind>`: 6 (the title's own) or 7 (FidelityFX).
pub fn asked_upscaler() -> Option<u32> {
    let spec = std::env::var("VICTORY_UPSCALER").ok()?;
    let kind = spec.trim().parse::<u32>().ok().filter(|k| matches!(k, 6 | 7));
    if kind.is_none() {
        eprintln!("VIctory: VICTORY_UPSCALER={spec}: expected 6 (the title's own) or 7 (FidelityFX)");
    }
    kind
}

/// What the switches ask for.
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Asked {
    pub output: Option<(u32, u32)>,
    pub render_scale: Option<f32>,
    pub ffx_scale: Option<f32>,
    pub upscaler: Option<u32>,
}

impl Asked {
    /// The switches, read from the environment (the output size against the
    /// profile's `modes`).
    pub fn from_env(modes: &[(u32, u32)]) -> Self {
        Self {
            output: asked_output(modes),
            render_scale: asked_render_scale(),
            ffx_scale: asked_ffx_scale(),
            upscaler: asked_upscaler(),
        }
    }

    pub fn any(&self) -> bool {
        *self != Self::default()
    }
}

/// One change asked for: what, the value, and the bytes to write (the offset in
/// the image of each) or why it cannot be done.
#[derive(Debug, PartialEq)]
pub struct Change {
    pub what: &'static str,
    pub value: String,
    pub writes: Result<Vec<(usize, Vec<u8>)>, String>,
}

const NO_PROFILE: &str = "no profile (see VICTORY_TITLE_PROFILE)";

/// The bytes for the edit `pick` takes out of the profile, at the one place its
/// pattern matches.
fn edit_writes(
    profile: Option<&Profile>,
    pick: impl Fn(&Profile) -> Option<&Edit>,
    key: &str,
    code: &Code,
    values: &Values,
) -> Result<Vec<(usize, Vec<u8>)>, String> {
    let profile = profile.ok_or(NO_PROFILE)?;
    let edit = pick(profile).ok_or_else(|| format!("the profile has no {key}"))?;
    let at = find_once(code, &edit.pattern)?;
    Ok(vec![(at, edit.code.encode(at, values).map_err(|e| format!("{key}.code: {e}"))?)])
}

/// What each switch asked for becomes: the places found in `code`, the bytes
/// for each. Nothing is written.
pub fn plan(profile: Option<&Profile>, code: &Code, asked: &Asked) -> Vec<Change> {
    let mut out = Vec::new();
    if let Some((width, height)) = asked.output {
        let (value, values) = (format!("{width}x{height}"), Values { width: Some(width), height: Some(height), scale: None });
        let writes = edit_writes(profile, |p| p.output.as_ref(), "resolution.output", code, &values);
        out.push(Change { what: "settings' output size applied", value: value.clone(), writes });
        // The pool check is sized for one output size, which leaves it as it is.
        if let Some(p) = profile {
            let skipped = p.pool.as_ref().is_some_and(|pool| pool.unless == Some((width, height)));
            if !skipped {
                let writes = edit_writes(profile, |p| p.pool.as_ref().map(|pool| &pool.edit), "resolution.pool", code, &values);
                out.push(Change { what: "back buffer pool's left-over check", value, writes });
            }
        }
    }
    if let Some(scale) = asked.render_scale {
        let values = Values { scale: Some(scale), ..Values::default() };
        let writes = edit_writes(profile, |p| p.own_scale.as_ref(), "resolution.own", code, &values);
        out.push(Change { what: "the title's own upscaler's render scale", value: format!("{scale}"), writes });
    }
    if let Some(scale) = asked.ffx_scale {
        let writes = (|| {
            let profile = profile.ok_or(NO_PROFILE)?;
            let ffx = profile.ffx.as_ref().ok_or("the profile has no resolution.ffx")?;
            let at = find_once(code, &ffx.pattern)?;
            // The scale goes into the padding; each load reads it from there.
            let mut writes = vec![(at + ffx.slot, scale.to_le_bytes().to_vec())];
            for load in &ffx.loads {
                let disp = i32::try_from(ffx.slot as i64 - (*load as i64 + 8)).map_err(|_| "a load is out of reach of the slot")?;
                writes.push((at + load + 4, disp.to_le_bytes().to_vec()));
            }
            Ok(writes)
        })();
        out.push(Change { what: "FidelityFX's render scale", value: format!("{scale}"), writes });
    }
    if let Some(kind) = asked.upscaler {
        let writes = (|| {
            let profile = profile.ok_or(NO_PROFILE)?;
            let pattern = profile.picker.as_ref().ok_or("the profile has no resolution.picker")?;
            let at = find_once(code, pattern)?;
            Ok(vec![(at, picker_code(kind))])
        })();
        out.push(Change { what: "upscaler picker", value: format!("returns {kind}"), writes });
    }
    out
}

/// `mov eax, kind ; ret`.
pub fn picker_code(kind: u32) -> Vec<u8> {
    let mut c = vec![0xb8];
    c.extend(kind.to_le_bytes());
    c.push(0xc3);
    c
}

/// Writes `code` at the offset `at` of the image of `size` bytes at `base`.
///
/// # Safety
/// The image stays mapped and no thread runs its code yet.
#[cfg(windows)]
pub unsafe fn write(base: usize, size: usize, at: usize, code: &[u8]) -> Patched {
    use windows_sys::Win32::System::Diagnostics::Debug::FlushInstructionCache;
    use windows_sys::Win32::System::Memory::{PAGE_EXECUTE_READWRITE, VirtualProtect};
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    if at.checked_add(code.len()).is_none_or(|end| end > size) {
        return Patched::Failed("outside the image");
    }
    let at = base + at;
    let mut old = 0;
    // SAFETY: inside the image (checked); made writable for this and put
    // back; nothing runs it yet.
    unsafe {
        if VirtualProtect(at as *const _, code.len(), PAGE_EXECUTE_READWRITE, &mut old) == 0 {
            return Patched::Failed("Windows would not make it writable");
        }
        std::ptr::copy_nonoverlapping(code.as_ptr(), at as *mut u8, code.len());
        VirtualProtect(at as *const _, code.len(), old, &mut old);
        FlushInstructionCache(GetCurrentProcess(), at as *const _, code.len());
    }
    Patched::Done
}

/// Applies what the switches ask for, as `profile` says where; a line each in
/// the log and on standard output. The image's executable sections are `code`.
///
/// # Safety
/// The image of `size` bytes at `base` stays mapped and no thread runs its
/// code yet.
#[cfg(windows)]
pub unsafe fn apply_asked(base: usize, size: usize, profile: Option<&Profile>, code: &Code) {
    let asked = Asked::from_env(profile.map_or(&[][..], |p| p.modes.as_slice()));
    for change in plan(profile, code, &asked) {
        let Change { what, value, writes } = change;
        match writes {
            Err(why) => {
                tracing::warn!(what, value, why, "resolution: not working");
                println!("resolution: {what} ({value}): not working: {why}");
            }
            Ok(writes) => {
                let mut done = Patched::Done;
                for (at, bytes) in &writes {
                    // SAFETY: the caller's promise.
                    let one = unsafe { write(base, size, *at, bytes) };
                    if one != Patched::Done {
                        done = one;
                        break;
                    }
                }
                let offset = format!("{:#x}", writes.first().map_or(0, |w| w.0));
                match done {
                    Patched::Done => tracing::info!(site = %offset, what, value, "resolution: patched"),
                    Patched::Failed(why) => tracing::warn!(site = %offset, what, why, "resolution: not patched"),
                }
                println!("resolution: {what} ({value}) at title+{offset}: {done:?}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switches_parse() {
        let modes = [(1280, 720), (1920, 1080)];
        assert_eq!(parse_output("1920x1080", &modes), Some((1920, 1080)));
        assert_eq!(parse_output(" 1280X720 ", &modes), Some((1280, 720)));
        assert_eq!(parse_output("1600x900", &modes), None);
        assert_eq!(parse_output("1080p", &modes), None);
        // Without a list of modes, any plausible size.
        assert_eq!(parse_output("1600x900", &[]), Some((1600, 900)));
        assert_eq!(parse_output("1x1", &[]), None);
        assert_eq!(parse_render_scale("0.67"), Some(0.67));
        assert_eq!(parse_render_scale("0.5"), Some(0.5));
        assert_eq!(parse_render_scale("1"), Some(1.0));
        assert_eq!(parse_render_scale("0.4"), None);
        assert_eq!(parse_render_scale("1.5"), None);
        assert_eq!(picker_code(7), [0xb8, 7, 0, 0, 0, 0xc3]);
    }

    /// Made-up bytes, not any title's.
    const PROFILE: &str = "\
        resolution.modes = 640x360 1280x720\n\
        resolution.output = 01 02 03 04 05 06 07 08 09 0A 0B 0C 0D 0E 0F 10 11 12 13 14\n\
        resolution.output.code = B8 {width} BF {height} E8 {rel:0x40} CC CC\n\
        resolution.pool = 21 22 23 24 25 26\n\
        resolution.pool.code = 21 22 23 EB 01\n\
        resolution.pool.unless = 1280x720\n\
        resolution.own = 31 32 33 34 35 36 37 38 39 3A\n\
        resolution.own.code = 31 32 33 {scale}\n\
        resolution.ffx = 41 42 43 44 45 46 47 48 49 4A 4B 4C 4D 4E 4F 50 51 52 53 54 55 56 57 58\n\
        resolution.ffx.loads = 0x2 0x6\n\
        resolution.ffx.slot = 0x14\n\
        resolution.picker = 61 62 ?? 64 65\n";

    /// The code of the made-up title: each pattern once.
    fn code() -> Vec<u8> {
        let mut code = vec![0x90u8; 0x100];
        code.extend((1..=0x14).collect::<Vec<u8>>()); // output, at +0x100
        code.extend([0x90; 3]);
        code.extend([0x21, 0x22, 0x23, 0x24, 0x25, 0x26]); // pool, at +0x117
        code.extend([0x90; 3]);
        code.extend((0x31..=0x3a).collect::<Vec<u8>>()); // own, at +0x120
        code.extend((0x41..=0x58).collect::<Vec<u8>>()); // ffx, at +0x12a
        code.extend([0x61, 0x62, 0xee, 0x64, 0x65]); // picker, at +0x142
        code
    }

    fn found(change: &Change) -> &Vec<(usize, Vec<u8>)> {
        change.writes.as_ref().unwrap_or_else(|e| panic!("{}: {e}", change.what))
    }

    #[test]
    fn what_is_asked_becomes_bytes_at_the_places_found() {
        let profile = Profile::parse(PROFILE).unwrap();
        let code = code();
        let code: &Code = &[(0x5000, &code)];
        let asked = Asked { output: Some((1920, 1080)), render_scale: Some(0.75), ffx_scale: Some(0.5), upscaler: Some(7) };
        let plan = plan(Some(&profile), code, &asked);
        assert_eq!(plan.len(), 5);
        // The output: the profile's template at the match, its rel32 to 0x40 from the end of its own bytes.
        let output = found(&plan[0]);
        let at = 0x5000 + 0x100;
        let mut want = vec![0xb8];
        want.extend(1920u32.to_le_bytes());
        want.push(0xbf);
        want.extend(1080u32.to_le_bytes());
        want.push(0xe8);
        want.extend((0x40 - (at as i32 + 15)).to_le_bytes());
        want.extend([0xcc, 0xcc]);
        assert_eq!(output, &vec![(at, want)]);
        // The pool check is left alone for the one size, and not for another.
        assert_eq!(plan[1].what, "back buffer pool's left-over check");
        assert_eq!(found(&plan[1]), &vec![(0x5000 + 0x117, vec![0x21, 0x22, 0x23, 0xeb, 0x01])]);
        let same = Asked { output: Some((1280, 720)), ..Asked::default() };
        let plan_same = super::plan(Some(&profile), code, &same);
        assert_eq!(plan_same.len(), 1, "no pool check change for that size");
        assert_eq!(plan_same[0].what, "settings' output size applied");
        // The scale: the template's {scale}.
        let mut own = vec![0x31, 0x32, 0x33];
        own.extend(0.75f32.to_le_bytes());
        assert_eq!(found(&plan[2]), &vec![(0x5000 + 0x120, own)]);
        // The other upscaler: the scale into the slot, both loads pointed at it.
        let ffx = 0x5000 + 0x12a;
        assert_eq!(
            found(&plan[3]),
            &vec![
                (ffx + 0x14, 0.5f32.to_le_bytes().to_vec()),
                (ffx + 2 + 4, (0x14i32 - (2 + 8)).to_le_bytes().to_vec()),
                (ffx + 6 + 4, (0x14i32 - (6 + 8)).to_le_bytes().to_vec()),
            ]
        );
        assert_eq!(found(&plan[4]), &vec![(0x5000 + 0x142, vec![0xb8, 7, 0, 0, 0, 0xc3])]);
    }

    #[test]
    fn without_a_profile_or_its_entry_or_its_code_nothing_is_changed_and_it_is_said() {
        let asked = Asked { output: Some((1920, 1080)), render_scale: Some(0.75), ffx_scale: Some(0.5), upscaler: Some(7) };
        let code = code();
        let code: &Code = &[(0x5000, &code)];
        // No profile.
        let none = plan(None, code, &asked);
        assert_eq!(none.len(), 4);
        for change in &none {
            let why = change.writes.as_ref().unwrap_err();
            assert!(why.contains("no profile") && why.contains("VICTORY_TITLE_PROFILE"), "{}: {why}", change.what);
        }
        // A profile without the entries.
        let profile = Profile::parse("resolution.picker = 61 62 ?? 64 65").unwrap();
        let some = plan(Some(&profile), code, &asked);
        let whys: Vec<String> = some.iter().map(|c| c.writes.as_ref().err().cloned().unwrap_or_default()).collect();
        assert!(whys[0].contains("the profile has no resolution.output"), "{whys:?}");
        assert!(whys[1].contains("the profile has no resolution.pool"), "{whys:?}");
        assert!(whys[2].contains("no resolution.own"), "{whys:?}");
        assert!(whys[3].contains("no resolution.ffx"), "{whys:?}");
        assert!(some[4].writes.is_ok());
        // Entries whose code is not in this build, or is twice.
        let other = Profile::parse(PROFILE).unwrap();
        let mut elsewhere = code_without_the_picker();
        let plan_missing = plan(Some(&other), &[(0x5000, &elsewhere)], &Asked { upscaler: Some(6), ..Asked::default() });
        assert!(plan_missing[0].writes.as_ref().unwrap_err().contains("not in this build"));
        elsewhere.extend([0x61, 0x62, 0x00, 0x64, 0x65, 0x61, 0x62, 0x01, 0x64, 0x65]);
        let plan_twice = plan(Some(&other), &[(0x5000, &elsewhere)], &Asked { upscaler: Some(6), ..Asked::default() });
        assert!(plan_twice[0].writes.as_ref().unwrap_err().contains("matches 2 places"));
        // Nothing asked, nothing planned.
        assert!(plan(Some(&other), code, &Asked::default()).is_empty());
        assert!(!Asked::default().any() && asked.any());
    }

    fn code_without_the_picker() -> Vec<u8> {
        let mut code = code();
        code.truncate(code.len() - 5);
        code
    }

    #[test]
    fn a_template_that_wants_what_was_not_asked_for_is_said() {
        // The width is asked for by the template and not given.
        let profile = Profile::parse("resolution.output = 01 02 03 04 05\nresolution.output.code = B8 {scale}").unwrap();
        let bytes = [1u8, 2, 3, 4, 5];
        let change = &plan(Some(&profile), &[(0, &bytes)], &Asked { output: Some((640, 360)), ..Asked::default() })[0];
        assert!(change.writes.as_ref().unwrap_err().contains("resolution.output.code: it needs the scale"));
    }

    #[cfg(windows)]
    #[test]
    fn a_write_is_made_only_inside_the_image() {
        use windows_sys::Win32::System::Memory::{MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_EXECUTE_READ, VirtualAlloc, VirtualFree};
        // SAFETY: fresh memory of our own, read-only to code as the image's is.
        unsafe {
            let place = VirtualAlloc(std::ptr::null(), 0x1000, MEM_COMMIT | MEM_RESERVE, 0x40) as *mut u8;
            assert!(!place.is_null());
            let mut old = 0;
            windows_sys::Win32::System::Memory::VirtualProtect(place.cast(), 0x1000, PAGE_EXECUTE_READ, &mut old);
            assert_eq!(write(place as usize, 0x1000, 0x106, &0.5f32.to_le_bytes()), Patched::Done);
            assert_eq!(std::slice::from_raw_parts(place.add(0x106), 4), 0.5f32.to_le_bytes());
            assert!(matches!(write(place as usize, 0x1000, 0xffe, &[1, 2, 3, 4]), Patched::Failed(_)));
            assert!(matches!(write(place as usize, 0x1000, usize::MAX, &[1]), Patched::Failed(_)));
            VirtualFree(place.cast(), 0, MEM_RELEASE);
        }
    }

    /// The switches are applied to code of ours: found, written, and the rest left.
    #[cfg(windows)]
    #[test]
    fn the_planned_bytes_are_written() {
        use windows_sys::Win32::System::Memory::{MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_EXECUTE_READWRITE, VirtualAlloc, VirtualFree};
        let profile = Profile::parse(PROFILE).unwrap();
        let bytes = code();
        // SAFETY: fresh memory of our own, the image of a test.
        unsafe {
            let place = VirtualAlloc(std::ptr::null(), 0x1000, MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE) as *mut u8;
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), place, bytes.len());
            let mapped = std::slice::from_raw_parts(place, bytes.len());
            let asked = Asked { upscaler: Some(7), ffx_scale: Some(0.75), ..Asked::default() };
            for change in plan(Some(&profile), &[(0, mapped)], &asked) {
                for (at, code) in change.writes.unwrap() {
                    assert_eq!(write(place as usize, 0x1000, at, &code), Patched::Done);
                }
            }
            assert_eq!(std::slice::from_raw_parts(place.add(0x142), 6), [0xb8, 7, 0, 0, 0, 0xc3]);
            assert_eq!(std::slice::from_raw_parts(place.add(0x12a + 0x14), 4), 0.75f32.to_le_bytes());
            assert_eq!(*place.add(0x100), 1, "what was not asked for stays");
            VirtualFree(place.cast(), 0, MEM_RELEASE);
        }
    }
}
