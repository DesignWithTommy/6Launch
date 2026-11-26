//! Byte patches written into a loaded module after relocation, before TLS
//! redirection and import binding. `VICTORY_PATCH=<module>:<rva>:<hex>,...`
//! writes anything; named switches (`VICTORY_NO_WATCHDOG=1`) write known
//! patches only where the bytes they replace match. Every write is logged.
//! The raw knob follows Halfect's upstream PR #1059.

use std::env;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Patch {
    // file name, case-insensitive; `None` tries every module
    module: Option<String>,
    rva: usize,
    // bytes that must be there first; `None` writes blind
    expect: Option<Vec<u8>>,
    bytes: Vec<u8>,
}

// the title's hang watchdog thread (+0x2215a60) kills the process when the
// main loop's heartbeat (+0x71b6338) is 30 s old; frames that slow are normal
// here. `jne` on its quit flag becomes `jmp`, so the thread exits at once
// through its own epilogue. image b99f876b (930.048.020).
const WATCHDOG_RVA: usize = 0x2215ab8;
const WATCHDOG_OLD: [u8; 14] = [
    0x8a, 0x05, 0x82, 0x08, 0xfa, 0x04, 0xa8, 0x01, 0x0f, 0x85, 0x08, 0x03, 0x00, 0x00,
];
const WATCHDOG_NEW: [u8; 14] = [
    0x8a, 0x05, 0x82, 0x08, 0xfa, 0x04, 0xa8, 0x01, 0x90, 0xe9, 0x08, 0x03, 0x00, 0x00,
];

fn watchdog() -> Patch {
    Patch {
        module: None,
        rva: WATCHDOG_RVA,
        expect: Some(WATCHDOG_OLD.to_vec()),
        bytes: WATCHDOG_NEW.to_vec(),
    }
}

// VICTORY_FFX_FORCE: the upscaler-type getter (+0x7e93f0) answers 7 (the fsr
// wrapper) without the series s and settings-bit tests
const FFX_TYPE_RVA: usize = 0x7e93f0;
const FFX_TYPE_OLD: [u8; 6] = [0x48, 0x83, 0xec, 0x28, 0x80, 0x3d];
const FFX_TYPE_NEW: [u8; 6] = [0xb8, 0x07, 0x00, 0x00, 0x00, 0xc3];
// the wrapper's render scale (+0x28ef6a4): both console branches become one
// constant from VICTORY_FFX_SCALE (default 2/3)
const FFX_SCALE_RVA: usize = 0x28ef6a4;
const FFX_SCALE_OLD: [u8; 22] = [
    0x84, 0xc0, 0x75, 0x0a, 0xc5, 0xfa, 0x10, 0x05, 0x28, 0x78, 0x6d, 0x03, 0xeb, 0x08, 0xc5, 0xfa,
    0x10, 0x05, 0xa2, 0xe2, 0x86, 0x03,
];

fn ffx_scale() -> f32 {
    env::var("VICTORY_FFX_SCALE")
        .ok()
        .and_then(|v| v.trim().parse::<f32>().ok())
        .filter(|s| s.is_finite())
        .map_or(2.0 / 3.0, |s| s.clamp(0.25, 1.0))
}

fn ffx_force(scale: f32) -> [Patch; 2] {
    // mov eax, imm32; vmovd xmm0, eax; nops
    let mut scaled = vec![0xb8];
    scaled.extend_from_slice(&scale.to_bits().to_le_bytes());
    scaled.extend_from_slice(&[0xc5, 0xf9, 0x6e, 0xc0]);
    scaled.resize(FFX_SCALE_OLD.len(), 0x90);
    [
        Patch { module: None, rva: FFX_TYPE_RVA, expect: Some(FFX_TYPE_OLD.to_vec()), bytes: FFX_TYPE_NEW.to_vec() },
        Patch { module: None, rva: FFX_SCALE_RVA, expect: Some(FFX_SCALE_OLD.to_vec()), bytes: scaled },
    ]
}

// VICTORY_SHADOW_CASCADES=2..3: the sun's cascades drawn. the clamp of the
// tunable count (+0x97dc50 into +0x84ddbe0) only sets the active count; the
// count the render thread loops over (+0x84ddbd4) is forced to 4 at +0x98070a
// (and the active one at +0x9807bb in shadow modes 1-2), so those go to n too
const CASCADES_RVA: usize = 0x97dc50;
const CASCADES_OLD: [u8; 8] = [0xb9, 0x04, 0x00, 0x00, 0x00, 0x83, 0xf8, 0x04];
// movabs rcx, 0x3f800000_00000004 (stored over +0x84ddbd4/+0x84ddbd8)
const CASCADES_DRAWN_RVA: usize = 0x98070a;
const CASCADES_DRAWN_OLD: [u8; 10] = [0x48, 0xb9, 0x04, 0, 0, 0, 0, 0, 0x80, 0x3f];
// mov dword [+0x84ddbdc], 4
const CASCADES_MODE_RVA: usize = 0x9807bb;
const CASCADES_MODE_OLD: [u8; 10] = [0xc7, 0x05, 0x17, 0xd4, 0xb5, 0x07, 0x04, 0, 0, 0];

fn cascades(n: u8) -> Option<[Patch; 3]> {
    let with = |rva: usize, old: &[u8], at: usize| {
        let mut bytes = old.to_vec();
        bytes[at] = n;
        Patch { module: None, rva, expect: Some(old.to_vec()), bytes }
    };
    (2..=3).contains(&n).then(|| {
        [
            Patch {
                module: None,
                rva: CASCADES_RVA,
                expect: Some(CASCADES_OLD.to_vec()),
                bytes: vec![0xb9, n, 0x00, 0x00, 0x00, 0x83, 0xf8, n],
            },
            with(CASCADES_DRAWN_RVA, &CASCADES_DRAWN_OLD, 2),
            with(CASCADES_MODE_RVA, &CASCADES_MODE_OLD, 6),
        ]
    })
}

// constants the switches below point code at: .rdata's zero tail past its
// virtual size (+0x67e1c0c), still inside the mapped page
const SLACK_RVA: usize = 0x67e1c10;
// [cascade scale x1, x0.001] for the lea, then 170 x scale, then the lod scale
const SLACK_CASCADE: usize = SLACK_RVA;
const SLACK_FAR: usize = SLACK_RVA + 8;
const SLACK_LOD: usize = SLACK_RVA + 12;

// a rip-relative operand's disp32 repointed at `target`; `at` is the
// instruction, `end` its length
fn repoint(at: usize, old: &[u8], end: usize, target: usize) -> Patch {
    let disp = (target as i64 - (at + end) as i64) as i32;
    let mut bytes = old.to_vec();
    bytes[end - 4..end].copy_from_slice(&disp.to_le_bytes());
    Patch { module: None, rva: at, expect: Some(old.to_vec()), bytes }
}

fn constant(at: usize, values: &[f32]) -> Patch {
    Patch {
        module: None,
        rva: at,
        expect: Some(vec![0; 4 * values.len()]),
        bytes: values.iter().flat_map(|v| v.to_le_bytes()).collect(),
    }
}

fn scale_var(name: &str, low: f32) -> Option<f32> {
    env::var(name)
        .ok()
        .and_then(|v| v.trim().parse::<f32>().ok())
        .filter(|s| s.is_finite())
        .map(|s| s.clamp(low, 1.0))
}

// VICTORY_SHADOW_DIST=0.25..1: the sun's cascade split distances (+0x97dc94
// picks 1 or 0.001 from +0x616be30 by a flag, times the four splits) and the
// last split's 170 m cap (+0x97dfb3) are scaled: smaller cascades, fewer
// casters in each
const SHADOW_DIST_LEA_RVA: usize = 0x97dc7a;
const SHADOW_DIST_LEA_OLD: [u8; 7] = [0x48, 0x8d, 0x0d, 0xaf, 0xe1, 0x7e, 0x05];
const SHADOW_FAR_RVA: usize = 0x97dfb3;
const SHADOW_FAR_OLD: [u8; 9] = [0xc4, 0xe2, 0x79, 0x18, 0x05, 0xc4, 0x38, 0x7e, 0x05];

fn shadow_dist(s: f32) -> [Patch; 4] {
    [
        constant(SLACK_CASCADE, &[s, 0.001 * s]),
        constant(SLACK_FAR, &[170.0 * s]),
        repoint(SHADOW_DIST_LEA_RVA, &SHADOW_DIST_LEA_OLD, 7, SLACK_CASCADE),
        repoint(SHADOW_FAR_RVA, &SHADOW_FAR_OLD, 9, SLACK_FAR),
    ]
}

// VICTORY_LOD_SCALE=0.3..1: the lod scales' per-frame sum (+0x3e571f0, out
// at +0xa15e100 and the per-level scales after it) starts from the global
// override (+0xab4a20c, 1 unless debug sets it); it reads a constant instead,
// so every lod switches nearer and far detail drops a level sooner
const LOD_RVA: usize = 0x3e57472;
const LOD_OLD: [u8; 8] = [0xc5, 0x7a, 0x10, 0x1d, 0x92, 0x2d, 0xcf, 0x06];

fn lod_scale(s: f32) -> [Patch; 2] {
    [constant(SLACK_LOD, &[s]), repoint(LOD_RVA, &LOD_OLD, 8, SLACK_LOD)]
}

fn hex(s: &str) -> Option<usize> {
    usize::from_str_radix(s.trim().trim_start_matches("0x"), 16).ok()
}

fn hex_bytes(s: &str) -> Option<Vec<u8>> {
    let s = s.trim().trim_start_matches("0x");
    if s.is_empty() || !s.len().is_multiple_of(2) || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

fn on(name: &str) -> bool {
    env::var(name).is_ok_and(|v| v == "1")
}

pub(crate) fn from_env() -> Vec<Patch> {
    let mut patches = parse(&env::var("VICTORY_PATCH").unwrap_or_default());
    if on("VICTORY_NO_WATCHDOG") {
        patches.push(watchdog());
    }
    if on("VICTORY_FFX_FORCE") {
        patches.extend(ffx_force(ffx_scale()));
    }
    if let Some(s) = scale_var("VICTORY_SHADOW_DIST", 0.25) {
        patches.extend(shadow_dist(s));
    }
    if let Some(s) = scale_var("VICTORY_LOD_SCALE", 0.3) {
        patches.extend(lod_scale(s));
    }
    if let Some(p) = env::var("VICTORY_SHADOW_CASCADES").ok().and_then(|v| v.trim().parse().ok()).and_then(cascades) {
        patches.extend(p);
    }
    patches
}

// malformed entries are skipped, like the other knobs
fn parse(spec: &str) -> Vec<Patch> {
    spec.split(',')
        .filter_map(|entry| {
            let mut parts = entry.splitn(3, ':');
            let module = parts.next()?.trim();
            let rva = hex(parts.next()?)?;
            let bytes = hex_bytes(parts.next()?)?;
            (!module.is_empty()).then(|| Patch {
                module: Some(module.into()),
                rva,
                expect: None,
                bytes,
            })
        })
        .collect()
}

fn planned<'a>(patches: &'a [Patch], module: &str, size: usize) -> Vec<&'a Patch> {
    patches
        .iter()
        .filter(|p| {
            p.module
                .as_ref()
                .is_none_or(|m| m.eq_ignore_ascii_case(module))
        })
        .filter(|p| {
            p.rva
                .checked_add(p.bytes.len())
                .is_some_and(|end| end <= size)
        })
        .collect()
}

// `target` is the patch's own range of the image (what `apply` maps); the
// replaced bytes, or what was found when it isn't what the patch expects
fn write(target: &mut [u8], patch: &Patch) -> Result<Vec<u8>, Vec<u8>> {
    let old = target.to_vec();
    if patch.expect.as_ref().is_some_and(|e| *e != old) {
        return Err(old);
    }
    target.copy_from_slice(&patch.bytes);
    Ok(old)
}

// a whole image in memory, as the tests have it
#[cfg(test)]
fn write_image(image: &mut [u8], patch: &Patch) -> Result<Vec<u8>, Vec<u8>> {
    write(&mut image[patch.rva..patch.rva + patch.bytes.len()], patch)
}

#[cfg(windows)]
pub(crate) fn apply(base: usize, size: usize, module: &str) {
    use std::ffi::c_void;

    use windows_sys::Win32::System::Memory::{PAGE_EXECUTE_READWRITE, VirtualProtect};

    let patches = from_env();
    let named = patches
        .iter()
        .filter(|p| {
            p.module
                .as_ref()
                .is_some_and(|m| m.eq_ignore_ascii_case(module))
        })
        .count();
    let planned = planned(&patches, module, size);
    let fitting = planned.iter().filter(|p| p.module.is_some()).count();
    if named > fitting {
        tracing::warn!(
            module,
            skipped = named - fitting,
            "VICTORY_PATCH entries outside the image, skipped"
        );
    }
    for patch in planned {
        let at = base + patch.rva;
        let len = patch.bytes.len();
        let mut protection = 0;
        // SAFETY: inside the mapped image (checked by `planned`); .text is
        // read-only, so lift the protection for the write and restore it.
        if unsafe {
            VirtualProtect(
                at as *const c_void,
                len,
                PAGE_EXECUTE_READWRITE,
                &mut protection,
            )
        } == 0
        {
            tracing::warn!(module, rva = format_args!("{:#x}", patch.rva), error = %std::io::Error::last_os_error(), "VirtualProtect failed; patch skipped");
            continue;
        }
        // SAFETY: in bounds of the mapped image, writable above.
        let result = write(
            unsafe { std::slice::from_raw_parts_mut(at as *mut u8, len) },
            patch,
        );
        let mut ignored = 0;
        // SAFETY: restores the range's own protection.
        unsafe { VirtualProtect(at as *const c_void, len, protection, &mut ignored) };
        match result {
            Ok(old) => tracing::info!(
                module,
                rva = format_args!("{:#x}", patch.rva),
                old = format_args!("{old:02x?}"),
                new = format_args!("{:02x?}", patch.bytes),
                "patch applied"
            ),
            // a known patch on another module or build: quiet unless named
            Err(found) if patch.module.is_some() => tracing::warn!(
                module,
                rva = format_args!("{:#x}", patch.rva),
                found = format_args!("{found:02x?}"),
                "patch skipped, bytes differ"
            ),
            Err(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ffx_scale_patch_keeps_its_length() {
        let [kind, scale] = ffx_force(0.5);
        assert_eq!(kind.bytes.len(), FFX_TYPE_OLD.len());
        assert_eq!(scale.bytes.len(), FFX_SCALE_OLD.len());
        assert_eq!(&scale.bytes[1..5], &0.5f32.to_bits().to_le_bytes());
    }

    #[test]
    fn cascades_patch_lowers_both_clamp_immediates_only() {
        let mut image = vec![0u8; CASCADES_MODE_RVA + 0x20];
        image[CASCADES_RVA..CASCADES_RVA + 8].copy_from_slice(&CASCADES_OLD);
        image[CASCADES_DRAWN_RVA..CASCADES_DRAWN_RVA + 10].copy_from_slice(&CASCADES_DRAWN_OLD);
        image[CASCADES_MODE_RVA..CASCADES_MODE_RVA + 10].copy_from_slice(&CASCADES_MODE_OLD);
        for p in cascades(2).unwrap() {
            assert!(write_image(&mut image, &p).is_ok());
        }
        assert_eq!(&image[CASCADES_RVA..CASCADES_RVA + 8], &[0xb9, 2, 0, 0, 0, 0x83, 0xf8, 2]);
        // the drawn count: only the immediate's low byte, the 1.0 beside it stays
        assert_eq!(&image[CASCADES_DRAWN_RVA..CASCADES_DRAWN_RVA + 10], &[0x48, 0xb9, 2, 0, 0, 0, 0, 0, 0x80, 0x3f]);
        assert_eq!(&image[CASCADES_MODE_RVA..CASCADES_MODE_RVA + 10], &[0xc7, 0x05, 0x17, 0xd4, 0xb5, 0x07, 2, 0, 0, 0]);
        assert!(cascades(1).is_none() && cascades(4).is_none());
    }

    // the shipped exe's bytes at an rva (VICTORY_TITLE_EXE or the usual copy), if it is there
    fn title_bytes(rva: usize, n: usize) -> Option<Vec<u8>> {
        let path = env::var("VICTORY_TITLE_EXE")
            .unwrap_or_else(|_| r"F:\download\GTA6.Xbox.v1.0.2.1\game_scarlett_master.exe".into());
        let mut f = std::fs::File::open(path).ok()?;
        use std::io::{Read, Seek, SeekFrom};
        let mut head = vec![0u8; 0x1000];
        f.read_exact(&mut head).ok()?;
        let u16_at = |o: usize| u16::from_le_bytes([head[o], head[o + 1]]) as usize;
        let u32_at = |o: usize| u32::from_le_bytes(head[o..o + 4].try_into().unwrap()) as usize;
        let pe = u32_at(0x3c);
        let sections = pe + 24 + u16_at(pe + 20);
        let at = (0..u16_at(pe + 6)).map(|i| sections + 40 * i).find(|&s| {
            let (va, raw) = (u32_at(s + 12), u32_at(s + 16));
            (va..va + raw).contains(&rva)
        })?;
        f.seek(SeekFrom::Start((rva - u32_at(at + 12) + u32_at(at + 20)) as u64)).ok()?;
        let mut out = vec![0u8; n];
        f.read_exact(&mut out).ok()?;
        Some(out)
    }

    #[test]
    fn known_patches_match_the_shipped_exe() {
        let Some(found) = title_bytes(CASCADES_RVA, 8) else {
            eprintln!("title exe not found, skipped");
            return;
        };
        assert_eq!(found, CASCADES_OLD);
        assert_eq!(title_bytes(CASCADES_DRAWN_RVA, 10).unwrap(), CASCADES_DRAWN_OLD);
        assert_eq!(title_bytes(CASCADES_MODE_RVA, 10).unwrap(), CASCADES_MODE_OLD);
        assert_eq!(title_bytes(FFX_TYPE_RVA, 6).unwrap(), FFX_TYPE_OLD);
        assert_eq!(title_bytes(SHADOW_DIST_LEA_RVA, 7).unwrap(), SHADOW_DIST_LEA_OLD);
        assert_eq!(title_bytes(SHADOW_FAR_RVA, 9).unwrap(), SHADOW_FAR_OLD);
        assert_eq!(title_bytes(LOD_RVA, 8).unwrap(), LOD_OLD);
        assert_eq!(title_bytes(SLACK_RVA, 16).unwrap(), vec![0; 16]);
        // the old operands' targets: the 1/0.001 pair, 170, the global lod scale
        let target = |at: usize, end: usize, old: &[u8]| (at + end) as i64 + i32::from_le_bytes(old[end - 4..end].try_into().unwrap()) as i64;
        assert_eq!(target(SHADOW_DIST_LEA_RVA, 7, &SHADOW_DIST_LEA_OLD), 0x616be30);
        assert_eq!(title_bytes(0x616be30, 8).unwrap(), [1.0f32.to_le_bytes(), 0.001f32.to_le_bytes()].concat());
        assert_eq!(title_bytes(target(SHADOW_FAR_RVA, 9, &SHADOW_FAR_OLD) as usize, 4).unwrap(), 170.0f32.to_le_bytes());
        assert_eq!(target(LOD_RVA, 8, &LOD_OLD), 0xab4a20c);
        assert_eq!(title_bytes(FFX_SCALE_RVA, FFX_SCALE_OLD.len()).unwrap(), FFX_SCALE_OLD);
    }

    #[test]
    fn distance_patches_point_their_operands_at_the_new_constants() {
        let p = shadow_dist(0.5);
        assert_eq!(p[0].bytes, [0.5f32.to_le_bytes(), 0.0005f32.to_le_bytes()].concat());
        assert_eq!(p[1].bytes, 85.0f32.to_le_bytes());
        for (patch, end, target) in [(&p[2], 7, SLACK_CASCADE), (&p[3], 9, SLACK_FAR)] {
            let disp = i32::from_le_bytes(patch.bytes[end - 4..end].try_into().unwrap());
            assert_eq!(patch.rva as i64 + end as i64 + disp as i64, target as i64);
            assert_eq!(patch.bytes[..end - 4], patch.expect.as_ref().unwrap()[..end - 4]);
        }
        let [value, load] = lod_scale(0.6);
        assert_eq!(value.bytes, 0.6f32.to_le_bytes());
        let disp = i32::from_le_bytes(load.bytes[4..8].try_into().unwrap());
        assert_eq!(LOD_RVA as i64 + 8 + disp as i64, SLACK_LOD as i64);
        assert_eq!(&load.bytes[..4], &LOD_OLD[..4]);
        // constants only land on zeros
        let mut image = vec![0u8; SLACK_RVA + 0x20];
        image[SLACK_LOD] = 1;
        assert!(write_image(&mut image, &value).is_err());
    }

    #[test]
    fn cascades_patch_refuses_other_bytes() {
        let mut image = vec![0x90u8; CASCADES_MODE_RVA + 0x20];
        let before = image.clone();
        for p in cascades(3).unwrap() {
            assert!(write_image(&mut image, &p).is_err());
        }
        assert_eq!(image, before);
    }

    #[test]
    fn parses_entries_like_the_other_knobs() {
        let patches = parse("game.exe:247af2d:90, Game.DLL : 0x1c8 : 90cc ");
        assert_eq!(
            patches,
            vec![
                Patch {
                    module: Some("game.exe".into()),
                    rva: 0x247af2d,
                    expect: None,
                    bytes: vec![0x90],
                },
                Patch {
                    module: Some("Game.DLL".into()),
                    rva: 0x1c8,
                    expect: None,
                    bytes: vec![0x90, 0xcc],
                },
            ]
        );
    }

    #[test]
    fn skips_malformed_entries() {
        let patches = parse(
            "game.exe:10, game.exe:zz:90, game.exe:10:9, game.exe:10:0x, game.exe:10:9g, :10:90",
        );
        assert!(patches.is_empty(), "{patches:?}");
        let patches = parse("game.exe:zz:90,game.exe:10:9091");
        assert_eq!(patches.len(), 1);
        assert_eq!(patches[0].bytes, vec![0x90, 0x91]);
        assert!(parse("").is_empty());
    }

    #[test]
    fn planned_matches_the_module_and_the_image_bounds() {
        let patches = parse("game.exe:0:90, GAME.EXE:e:9091, game.exe:f:9090, other.exe:0:90");
        let picked = planned(&patches, "game.exe", 0x10);
        assert_eq!(picked.len(), 2);
        assert_eq!(picked[0].rva, 0);
        assert_eq!(picked[1].rva, 0xe);
        assert_eq!(planned(&patches, "Game.Exe", 0x10).len(), 2);
        assert!(planned(&patches, "unrelated.exe", 0x10).is_empty());
    }

    #[test]
    fn planned_cannot_overflow_the_image_end() {
        let patches = parse(&format!("game.exe:{:x}:9090", usize::MAX - 1));
        assert!(planned(&patches, "game.exe", 0x10).is_empty());
    }

    #[test]
    fn write_replaces_and_reports_the_old_bytes() {
        let patches = parse("game.exe:4:010203");
        let planned = planned(&patches, "game.exe", 16);
        let mut image = [0xccu8; 16];
        assert_eq!(write_image(&mut image, planned[0]), Ok(vec![0xcc; 3]));
        assert_eq!(&image[4..7], &[1, 2, 3]);
        assert_eq!(image[7], 0xcc);
    }

    #[test]
    fn watchdog_patch_turns_the_quit_check_into_a_jump_to_the_same_exit() {
        let mut image = vec![0u8; WATCHDOG_RVA + 0x40];
        image[WATCHDOG_RVA..WATCHDOG_RVA + 14].copy_from_slice(&WATCHDOG_OLD);
        let patch = watchdog();
        let picked = planned(
            std::slice::from_ref(&patch),
            "victory-image-anything.dll",
            image.len(),
        );
        assert_eq!(picked.len(), 1);
        assert_eq!(write_image(&mut image, picked[0]), Ok(WATCHDOG_OLD.to_vec()));
        let at = WATCHDOG_RVA + 8;
        // jne rel32 (0f 85, ends at +6) -> nop; jmp rel32 (e9, ends at +6)
        assert_eq!(&image[at..at + 2], &[0x90, 0xe9]);
        let rel = i32::from_le_bytes(image[at + 2..at + 6].try_into().unwrap());
        assert_eq!(at as i64 + 6 + rel as i64, 0x2215dce);
    }

    #[test]
    fn write_works_on_the_patch_range_alone_as_apply_maps_it() {
        // up to aeb98b54 this indexed the 14-byte range by the rva and panicked (run s4-skip-1)
        let mut range = WATCHDOG_OLD;
        assert_eq!(write(&mut range, &watchdog()), Ok(WATCHDOG_OLD.to_vec()));
        assert_eq!(range, WATCHDOG_NEW);
    }

    #[test]
    fn watchdog_patch_leaves_other_bytes_alone() {
        let mut image = vec![0x90u8; WATCHDOG_RVA + 0x40];
        let before = image.clone();
        assert!(write_image(&mut image, &watchdog()).is_err());
        assert_eq!(image, before);
        assert!(planned(&[watchdog()], "small.dll", 0x1000).is_empty());
    }
}
