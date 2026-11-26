//! Making a title pick the host's FidelityFX upscaler (`VICTORY_FSR`).
//!
//! A console title that upscales with FidelityFX on one console model and
//! with its own temporal pass on another picks between them in its code. To
//! have it pick FidelityFX on the model that does not, two places in its
//! code are changed in the prepared copy in memory (the title's files are
//! never touched): the function that picks the upscaler is made to answer
//! FidelityFX, and the two loads of the render scale are pointed at the scale
//! of the preset (the function otherwise renders at the full output size).
//!
//! Nothing about a title is built into this program. Where the code to change
//! is, is told by a profile the user keeps (`VICTORY_FSR_PROFILE`, or
//! `[paths].fsr_profile`), by byte patterns that must each match exactly once
//! in the executable sections: a pattern that matches nowhere or twice (another
//! build of the title) leaves the title alone. The profile is a text file of
//! `key = value` lines, `#` starting a comment:
//!
//! ```text
//! # Only for this build of the executable (BLAKE3 of the file, as `run` prints it).
//! build = <64 hex digits>
//! # The function that picks the upscaler: replaced by `mov eax, <kind>; ret`.
//! picker = 48 83 ?? ?? ... (hex bytes, `??` for any byte)
//! # The function that sets the render size from a scale, whose two loads of
//! # the scale (`vmovss xmm, [rip+disp32]`, 8 bytes each) are at these offsets
//! # from the start of the match, and whose 4 padding bytes (0xCC) at `scale.slot`
//! # take the new scale.
//! scale = 41 56 ?? ...
//! scale.loads = 0x28 0x32
//! scale.slot = 0x64
//! # Optional, any number: bytes written at an offset of the scale function
//! # (on bytes its pattern fixes), `{mode}` being the preset's quality mode
//! # (1 quality, 2 balanced, 3 performance), for code that records the mode.
//! scale.edit = 0x1a b8 {mode} 00 00 00 ...
//! ```
//!
//! The idea (patterns found at launch, written only when found exactly once,
//! the render scale first) is Marvisak/VIctory PR 1793's (GPL-3.0); DexNom/VIctory
//! pins the same two places by address for one build. The profile file is this
//! project's own, so that no code of a title is kept in its source.

use anyhow::{Context, Result, bail, ensure};
use victory_loader::PreparedImage;

/// The upscaler kind the picker answers: AMD FidelityFX.
const FIDELITYFX: u32 = 7;
const EXECUTE: u32 = 0x2000_0000;

/// A parsed profile.
#[derive(Debug, Default, PartialEq)]
pub struct Profile {
    pub build: Option<String>,
    picker: Option<Vec<Option<u8>>>,
    scale: Option<ScalePlace>,
}

#[derive(Debug, PartialEq)]
struct ScalePlace {
    pattern: Vec<Option<u8>>,
    loads: Vec<usize>,
    slot: usize,
    edits: Vec<(usize, Vec<Edit>)>,
}

/// A byte of an edit: given, or the preset's quality mode.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Edit {
    Byte(u8),
    Mode,
}

fn number(text: &str) -> Result<usize> {
    let text = text.trim();
    match text.strip_prefix("0x") {
        Some(hex) => usize::from_str_radix(hex, 16),
        None => text.parse(),
    }
    .with_context(|| format!("not a number: {text}"))
}

/// A pattern of hex bytes and `??` (any byte), separated by spaces.
pub fn parse_pattern(text: &str) -> Result<Vec<Option<u8>>> {
    let pattern = text
        .split_whitespace()
        .map(|b| {
            if b == "??" {
                Ok(None)
            } else {
                u8::from_str_radix(b, 16)
                    .map(Some)
                    .with_context(|| format!("not a hex byte or ??: {b}"))
            }
        })
        .collect::<Result<Vec<_>>>()?;
    ensure!(
        pattern.iter().any(Option::is_some),
        "a pattern needs at least one fixed byte"
    );
    Ok(pattern)
}

/// `<offset> <byte>...`: bytes written at `offset` of the scale function,
/// each a hex byte or `{mode}` (the preset's `FFX_UPSCALE_QUALITY_MODE`: 1,
/// 2 or 3), for code that records which mode the title renders in.
fn parse_edit(text: &str) -> Result<(usize, Vec<Edit>)> {
    let mut words = text.split_whitespace();
    let at = number(words.next().context("scale.edit needs an offset")?)?;
    let bytes = words
        .map(|w| {
            if w == "{mode}" {
                Ok(Edit::Mode)
            } else {
                u8::from_str_radix(w, 16)
                    .map(Edit::Byte)
                    .with_context(|| format!("not a hex byte or {{mode}}: {w}"))
            }
        })
        .collect::<Result<Vec<_>>>()?;
    ensure!(!bytes.is_empty(), "scale.edit needs bytes");
    Ok((at, bytes))
}

impl Profile {
    pub fn parse(text: &str) -> Result<Self> {
        let mut build = None;
        let (mut picker, mut scale) = (None, None);
        let (mut loads, mut slot) = (None, None);
        let mut edits = Vec::new();
        for (n, line) in text.lines().enumerate() {
            let line = line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let at = || format!("profile line {}", n + 1);
            let (key, value) = line
                .split_once('=')
                .with_context(|| format!("{}: expected key = value", at()))?;
            let value = value.trim();
            match key.trim() {
                "build" => {
                    ensure!(
                        value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit()),
                        "{}: build is 64 hex digits",
                        at()
                    );
                    build = Some(value.to_ascii_lowercase());
                }
                "picker" => picker = Some(parse_pattern(value).with_context(at)?),
                "scale" => scale = Some(parse_pattern(value).with_context(at)?),
                "scale.loads" => {
                    loads = Some(
                        value
                            .split_whitespace()
                            .map(number)
                            .collect::<Result<Vec<_>>>()
                            .with_context(at)?,
                    );
                }
                "scale.slot" => slot = Some(number(value).with_context(at)?),
                "scale.edit" => edits.push(parse_edit(value).with_context(at)?),
                other => bail!("{}: unknown key {other}", at()),
            }
        }
        ensure!(
            scale.is_some() || edits.is_empty(),
            "scale.edit needs the scale pattern"
        );
        let scale = match (scale, loads, slot) {
            (None, None, None) => None,
            (Some(pattern), Some(loads), Some(slot)) => {
                ensure!(!loads.is_empty(), "scale.loads names no load");
                ensure!(
                    slot + 4 <= pattern.len() && loads.iter().all(|l| l + 8 <= pattern.len()),
                    "scale.loads and scale.slot must lie inside the scale pattern"
                );
                for (at, bytes) in &edits {
                    // What is overwritten is code the pattern pins down, not an open byte.
                    ensure!(
                        pattern
                            .get(*at..*at + bytes.len())
                            .is_some_and(|window| window.iter().all(Option::is_some)),
                        "scale.edit at {at:#x} must lie on fixed bytes of the scale pattern"
                    );
                }
                Some(ScalePlace {
                    pattern,
                    loads,
                    slot,
                    edits,
                })
            }
            _ => bail!("scale, scale.loads and scale.slot go together"),
        };
        if let Some(p) = &picker {
            ensure!(p.len() >= 6, "the picker pattern is shorter than its 6-byte replacement");
        }
        ensure!(
            picker.is_some(),
            "a profile needs a picker pattern (nothing else makes the title choose FidelityFX)"
        );
        Ok(Self {
            build,
            picker,
            scale,
        })
    }
}

/// Code to write at file offset `at`, found and checked.
#[derive(Debug, PartialEq)]
struct Patch {
    what: String,
    rva: u32,
    at: usize,
    bytes: Vec<u8>,
}

/// What `apply` did.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    /// Both places were changed (the scale only if the profile names it).
    Applied { scale: bool },
    /// Nothing was changed, for this reason.
    Skipped(String),
}

/// Changes the title's code in `image` for the upscaler picker to answer
/// FidelityFX and (if the profile has it) the render scale to be `scale`,
/// with `mode` (the preset's quality mode) where the profile's edits name it.
/// Everything is found before anything is written; with a place missing or
/// ambiguous nothing is written.
pub fn apply(image: &mut PreparedImage, profile: &Profile, scale: f32, mode: u32) -> Outcome {
    if let Some(build) = &profile.build
        && *build != image.original_hash.to_hex().as_str()
    {
        return Outcome::Skipped("the profile is for another build of the title".into());
    }
    let sections: Vec<(u32, usize, usize)> = image
        .info
        .sections
        .iter()
        .filter(|s| s.characteristics & EXECUTE != 0)
        .map(|s| {
            let len = match s.virtual_size {
                0 => s.file_size,
                vs => vs.min(s.file_size),
            };
            (s.rva, s.file_offset as usize, len as usize)
        })
        .collect();
    let plan = (|| -> Result<Vec<Patch>> {
        let mut patches = Vec::new();
        if let Some(place) = &profile.scale {
            let (rva, at) = find_once(&image.bytes, &sections, &place.pattern, "the render scale")?;
            let mut bytes = image.bytes[at..at + place.pattern.len()].to_vec();
            ensure!(
                patch_scale(&mut bytes, &place.loads, place.slot, scale),
                "the render scale at {rva:#x} is not the code the profile describes"
            );
            for (at, edit) in &place.edits {
                for (i, byte) in edit.iter().enumerate() {
                    bytes[at + i] = match byte {
                        Edit::Byte(b) => *b,
                        Edit::Mode => u8::try_from(mode).context("the quality mode is a byte")?,
                    };
                }
            }
            patches.push(Patch {
                what: format!("render scale {scale}"),
                rva,
                at,
                bytes,
            });
        }
        let picker = profile.picker.as_ref().context("no picker")?;
        let (rva, at) = find_once(&image.bytes, &sections, picker, "the upscaler picker")?;
        patches.push(Patch {
            what: format!("upscaler picker (answers {FIDELITYFX})"),
            rva,
            at,
            bytes: picker_stub(FIDELITYFX).to_vec(),
        });
        Ok(patches)
    })();
    let patches = match plan {
        Ok(patches) => patches,
        Err(why) => return Outcome::Skipped(format!("{why:#}")),
    };
    let scaled = patches.len() == 2;
    for p in patches {
        image.bytes[p.at..p.at + p.bytes.len()].copy_from_slice(&p.bytes);
        println!("patch          {:#x}: {}", p.rva, p.what);
    }
    Outcome::Applied { scale: scaled }
}

/// The (rva, file offset) of the one match of `pattern` in the executable
/// sections.
fn find_once(
    bytes: &[u8],
    sections: &[(u32, usize, usize)],
    pattern: &[Option<u8>],
    what: &str,
) -> Result<(u32, usize)> {
    let mut found = Vec::new();
    for &(rva, offset, len) in sections {
        let Some(code) = offset.checked_add(len).and_then(|end| bytes.get(offset..end)) else {
            bail!("cannot scan an executable section for {what}");
        };
        found.extend(
            matches(code, pattern)
                .into_iter()
                .map(|m| (rva + m as u32, offset + m)),
        );
    }
    match found[..] {
        [one] => Ok(one),
        [] => bail!("{what}: its code is not in this build of the title"),
        _ => bail!("{what}: its pattern matches {} places in this build", found.len()),
    }
}

/// The offsets in `code` where `pattern` matches.
fn matches(code: &[u8], pattern: &[Option<u8>]) -> Vec<usize> {
    // Look for its longest fixed run, then check the rest around each hit.
    let (anchor_at, anchor) = longest_fixed_run(pattern);
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(hit) = find_bytes(&code[from..], &anchor) {
        let hit = from + hit;
        from = hit + 1;
        let Some(start) = hit.checked_sub(anchor_at) else {
            continue;
        };
        let fits = code.get(start..start + pattern.len()).is_some_and(|window| {
            window
                .iter()
                .zip(pattern)
                .all(|(b, p)| p.is_none_or(|p| p == *b))
        });
        if fits {
            out.push(start);
        }
    }
    out
}

/// The first place `needle` occurs in `hay`.
fn find_bytes(hay: &[u8], needle: &[u8]) -> Option<usize> {
    let (&first, rest) = needle.split_first()?;
    let mut at = 0;
    while let Some(i) = hay.get(at..)?.iter().position(|&b| b == first) {
        let start = at + i;
        if hay.get(start + 1..start + needle.len()) == Some(rest) {
            return Some(start);
        }
        at = start + 1;
    }
    None
}

/// Where the longest run of fixed bytes in `pattern` starts, and its bytes.
fn longest_fixed_run(pattern: &[Option<u8>]) -> (usize, Vec<u8>) {
    let (mut best, mut start) = ((0, 0), 0);
    for (i, p) in pattern.iter().enumerate() {
        if p.is_none() {
            start = i + 1;
        } else if i + 1 - start > best.1 {
            best = (start, i + 1 - start);
        }
    }
    let run = pattern[best.0..best.0 + best.1].iter().flatten().copied();
    (best.0, run.collect())
}

/// `mov eax, kind; ret`.
fn picker_stub(kind: u32) -> [u8; 6] {
    let mut stub = [0xB8, 0, 0, 0, 0, 0xC3];
    stub[1..5].copy_from_slice(&kind.to_le_bytes());
    stub
}

/// Points the loads of the render scale in `code` (each `vmovss xmm,
/// [rip+disp32]`, 8 bytes, at an offset of `loads`) at `scale`, written into
/// the `int3` padding at `slot`. False, with nothing written, for other code.
fn patch_scale(code: &mut [u8], loads: &[usize], slot: usize, scale: f32) -> bool {
    const VMOVSS_RIP: [u8; 3] = [0xC5, 0xFA, 0x10];
    let load_ok = |l: usize| {
        code.get(l..l + 8).is_some_and(|i| {
            i[..3] == VMOVSS_RIP
                // mod 00, r/m 101: RIP-relative, whatever the register.
                && i[3] & 0xC7 == 0x05
        })
    };
    if !loads.iter().all(|&l| load_ok(l)) || code.get(slot..slot + 4) != Some(&[0xCC; 4]) {
        return false;
    }
    code[slot..slot + 4].copy_from_slice(&scale.to_le_bytes());
    for &l in loads {
        let disp = slot as i64 - (l as i64 + 8);
        code[l + 4..l + 8].copy_from_slice(&(disp as i32).to_le_bytes());
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    // Synthetic code: no title's bytes anywhere here.
    const PICKER: &str = "55 48 89 E5 ?? ?? 90 90 ?? 5D C3";
    const SCALE: &str = "11 22 33 44 55 66 77 88 \
                         C5 FA 10 05 ?? ?? ?? ?? 90 90 \
                         C5 FA 10 0D ?? ?? ?? ?? 99 \
                         CC CC CC CC CC CC";
    // Offsets in SCALE: the loads at 8 and 18, the padding from 27.
    const PROFILE: &str = "# a made-up profile\n\
        picker = 55 48 89 E5 ?? ?? 90 90 ?? 5D C3   # comment\n\
        scale = 11 22 33 44 55 66 77 88 C5 FA 10 05 ?? ?? ?? ?? 90 90 C5 FA 10 0D ?? ?? ?? ?? 99 CC CC CC CC CC CC\n\
        scale.loads = 8 18\n\
        scale.slot = 0x1b\n";

    fn instance(pattern: &str, fill: u8) -> Vec<u8> {
        parse_pattern(pattern)
            .unwrap()
            .into_iter()
            .map(|p| p.unwrap_or(fill))
            .collect()
    }

    /// An executable image of our own: code in a section at file offset 0x200.
    fn image(code: &[u8]) -> PreparedImage {
        let mut bytes = vec![0u8; 0x200];
        bytes.extend_from_slice(code);
        let info = victory_pe::ImageInfo {
            machine: 0x8664,
            characteristics: 0,
            timestamp: 0,
            subsystem: 0,
            dll_characteristics: 0,
            image_base: 0x1_4000_0000,
            size_of_image: 0x2000,
            entry_rva: 0,
            stack_reserve: 0,
            stack_commit: 0,
            sections: vec![victory_pe::SectionInfo {
                name: ".text".into(),
                rva: 0x1000,
                virtual_size: code.len() as u32,
                file_offset: 0x200,
                file_size: code.len() as u32,
                characteristics: EXECUTE | 0x4000_0000,
            }],
            imports: Vec::new(),
            delay_imports: Vec::new(),
            exports: Vec::new(),
            tls: None,
            has_relocations: false,
            has_exception_table: false,
            has_load_config: false,
            codeview: None,
        };
        PreparedImage {
            info,
            bytes,
            original_hash: blake3::hash(b"a made-up executable"),
        }
    }
    #[test]
    fn a_profile_parses_and_names_what_is_wrong_with_a_bad_one() {
        let profile = Profile::parse(PROFILE).unwrap();
        assert_eq!(profile.build, None);
        let scale = profile.scale.unwrap();
        assert_eq!((scale.loads.clone(), scale.slot), (vec![8, 18], 27));
        assert_eq!(scale.pattern.len(), 33);
        assert!(Profile::parse("picker = 01 02 03 04 05 06 07").is_ok());
        for (bad, why) in [
            ("scale = 11", "scale, scale.loads and scale.slot go together"),
            ("picker = 11 22", "shorter than its 6-byte replacement"),
            ("picker = ?? ?? ?? ?? ?? ?? ??", "at least one fixed byte"),
            ("picker = 1 2 3 4 5 6 zz", "not a hex byte"),
            ("colour = 1", "unknown key"),
            ("picker", "expected key = value"),
            ("build = 12", "64 hex digits"),
            ("scale = 11 22\nscale.loads = 0\nscale.slot = 0", "inside the scale pattern"),
            ("", "needs a picker"),
        ] {
            let error = format!("{:#}", Profile::parse(bad).unwrap_err());
            assert!(error.contains(why), "{bad:?}: {error}");
        }
    }

    #[test]
    fn a_pattern_is_found_with_any_open_bytes() {
        let mut code = vec![0x90u8; 0x80];
        code[0x23..0x23 + 11].copy_from_slice(&instance(PICKER, 0xAB));
        let pattern = parse_pattern(PICKER).unwrap();
        assert_eq!(matches(&code, &pattern), [0x23]);
        // A changed fixed byte is not a match.
        code[0x23 + 10] = 0xC2;
        assert!(matches(&code, &pattern).is_empty());
        // Cut off at either end is not one.
        let whole = instance(PICKER, 0);
        assert!(matches(&whole[1..], &pattern).is_empty());
        assert!(matches(&whole[..whole.len() - 1], &pattern).is_empty());
    }

    #[test]
    fn the_longest_fixed_run_anchors_the_search() {
        let (at, run) = longest_fixed_run(&parse_pattern("01 ?? 02 03 ?? 04 05 06 ??").unwrap());
        assert_eq!((at, run), (5, vec![4, 5, 6]));
        assert_eq!(find_bytes(b"abcabd", b"abd"), Some(3));
        assert_eq!(find_bytes(b"abc", b"abcd"), None);
    }

    #[test]
    fn the_picker_and_the_scale_are_written_where_the_patterns_are() {
        let profile = Profile::parse(PROFILE).unwrap();
        let mut code = vec![0x90u8; 0x40];
        code.extend(instance(PICKER, 0x11));
        code.extend([0xAA; 5]);
        let scale_at = code.len();
        code.extend(instance(SCALE, 0x00));
        code.extend([0x90; 9]);
        let mut image = image(&code);
        let outcome = apply(&mut image, &profile, 0.5, 1);
        assert_eq!(outcome, Outcome::Applied { scale: true });
        // The picker answers 7.
        assert_eq!(
            image.bytes[0x200 + 0x40..0x200 + 0x40 + 6],
            [0xB8, 7, 0, 0, 0, 0xC3]
        );
        // The scale sits in the padding, and both loads point at it.
        let base = 0x200 + scale_at;
        assert_eq!(image.bytes[base + 27..base + 31], 0.5f32.to_le_bytes());
        for load in [8usize, 18] {
            let disp = i32::from_le_bytes(image.bytes[base + load + 4..base + load + 8].try_into().unwrap());
            assert_eq!(load as i32 + 8 + disp, 27);
        }
        // The register of the second load is untouched.
        assert_eq!(image.bytes[base + 18..base + 22], [0xC5, 0xFA, 0x10, 0x0D]);
    }

    #[test]
    fn nothing_is_written_unless_both_places_are_found_exactly_once() {
        let profile = Profile::parse(PROFILE).unwrap();
        let picker = instance(PICKER, 0x11);
        let scale = instance(SCALE, 0x00);
        // The scale missing.
        let mut image1 = image(&[picker.clone(), vec![0x90; 8]].concat());
        let before = image1.bytes.clone();
        assert!(matches!(apply(&mut image1, &profile, 0.5, 1), Outcome::Skipped(why) if why.contains("render scale")));
        assert_eq!(image1.bytes, before);
        // The picker twice.
        let mut image2 = image(&[picker.clone(), vec![0x90; 3], picker.clone(), scale.clone()].concat());
        let before = image2.bytes.clone();
        assert!(matches!(apply(&mut image2, &profile, 0.5, 1), Outcome::Skipped(why) if why.contains("matches 2 places")));
        assert_eq!(image2.bytes, before);
        // Already patched (no padding left): the code is not what the profile
        // describes.
        let mut once = image(&[picker, scale].concat());
        assert_eq!(apply(&mut once, &profile, 0.5, 1), Outcome::Applied { scale: true });
        let patched = once.bytes[0x200..].to_vec();
        let mut again = image(&patched);
        assert!(matches!(apply(&mut again, &profile, 0.25, 1), Outcome::Skipped(_)));
    }

    #[test]
    fn a_profile_for_another_build_leaves_the_title_alone() {
        let text = format!("build = {}\n{PROFILE}", "ab".repeat(32));
        let profile = Profile::parse(&text).unwrap();
        let mut code = instance(PICKER, 0x11);
        code.extend(instance(SCALE, 0x00));
        let mut image = image(&code);
        let before = image.bytes.clone();
        assert!(matches!(apply(&mut image, &profile, 0.5, 1), Outcome::Skipped(why) if why.contains("another build")));
        assert_eq!(image.bytes, before);
        // For this one, it applies.
        let own = format!("build = {}\n{PROFILE}", image.original_hash.to_hex());
        assert_eq!(
            apply(&mut image, &Profile::parse(&own).unwrap(), 0.5, 1),
            Outcome::Applied { scale: true }
        );
    }

    #[test]
    fn an_edit_writes_the_presets_mode_where_the_pattern_pins_the_code() {
        let profile = Profile::parse(&format!("{PROFILE}scale.edit = 2 AA {{mode}} BB\n")).unwrap();
        let mut code = instance(PICKER, 0x11);
        let scale_at = code.len();
        code.extend(instance(SCALE, 0x00));
        let mut image = image(&code);
        assert_eq!(
            apply(&mut image, &profile, 0.5, 3),
            Outcome::Applied { scale: true }
        );
        let base = 0x200 + scale_at;
        assert_eq!(image.bytes[base + 2..base + 5], [0xAA, 3, 0xBB]);
        // The bytes around it are the pattern's own.
        assert_eq!(image.bytes[base..base + 2], [0x11, 0x22]);
        assert_eq!(image.bytes[base + 5], 0x66);
        for (bad, why) in [
            // Offsets 12 to 15 of the scale pattern are open bytes.
            ("scale.edit = 12 AA\n", "fixed bytes"),
            ("scale.edit = 0x20 AA BB CC DD EE\n", "fixed bytes"),
            ("scale.edit = 2\n", "needs bytes"),
            ("scale.edit = 2 zz\n", "not a hex byte"),
        ] {
            let error = format!("{:#}", Profile::parse(&format!("{PROFILE}{bad}")).unwrap_err());
            assert!(error.contains(why), "{bad:?}: {error}");
        }
        let alone = "picker = 55 48 89 E5 ?? ?? 90 90 ?? 5D C3\nscale.edit = 2 AA\n";
        assert!(format!("{:#}", Profile::parse(alone).unwrap_err()).contains("needs the scale pattern"));
    }

    #[test]
    fn a_picker_alone_is_applied_without_the_scale() {
        let profile = Profile::parse("picker = 55 48 89 E5 ?? ?? 90 90 ?? 5D C3").unwrap();
        let mut image = image(&instance(PICKER, 0x11));
        assert_eq!(apply(&mut image, &profile, 0.5, 1), Outcome::Applied { scale: false });
    }

    /// The user's own profile against the user's own title: the executable
    /// (`VICTORY_TITLE_IMAGE`) is read and patched in memory only, nothing
    /// runs. Skipped unless both it and `VICTORY_FSR_PROFILE` are set.
    #[test]
    fn a_users_profile_applies_to_a_users_title() {
        let (Some(image), Some(profile)) = (
            std::env::var_os("VICTORY_TITLE_IMAGE"),
            std::env::var_os("VICTORY_FSR_PROFILE"),
        ) else {
            return;
        };
        let profile = Profile::parse(&std::fs::read_to_string(profile).unwrap()).unwrap();
        let prepared = victory_loader::prepare(&std::fs::read(image).unwrap()).unwrap();
        println!("build {}", prepared.original_hash.to_hex());
        for scale in [2.0f32 / 3.0, 1.0 / 1.7, 0.5] {
            let mut copy = PreparedImage {
                info: prepared.info.clone(),
                bytes: prepared.bytes.clone(),
                original_hash: prepared.original_hash,
            };
            let outcome = apply(&mut copy, &profile, scale, 1);
            assert_eq!(outcome, Outcome::Applied { scale: true }, "{outcome:?}");
            assert_ne!(copy.bytes, prepared.bytes);
        }
    }

    #[test]
    fn other_code_is_left_alone() {
        let mut code = vec![0xCCu8; 40];
        assert!(!patch_scale(&mut code, &[8, 18], 27, 0.5));
        assert!(code.iter().all(|&b| b == 0xCC));
        assert_eq!(picker_stub(7), [0xB8, 7, 0, 0, 0, 0xC3]);
    }
}
