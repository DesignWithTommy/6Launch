//! The user's profile of the title: where, in its code, the few places are that
//! this program changes in memory at load, and what goes there.
//!
//! Nothing about the title's code is kept in this repository: no offset, no
//! byte, no instruction pattern. The changes the program can make (renderer
//! port 2, the frame rate, the output size and the render scale) are told
//! *where* by a text file the user keeps outside the repository, given by
//! `VICTORY_TITLE_PROFILE` or `[paths].title_profile` of `victory.toml`.
//! Without it each of those features says so in the log (`not working: no
//! profile`) and stays off; the title is then run as it is.
//!
//! The file is `key = value` lines, `#` starting a comment. A *pattern* is hex
//! bytes with `??` for any byte, separated by spaces; it must match exactly
//! once in the executable sections of the mapped image: a pattern that matches
//! nowhere or twice (another build of the title) leaves that place alone and
//! says so. What the keys stand for (every number is decimal, or hex with a
//! `0x`):
//!
//! ```text
//! # Only for this build of the executable (BLAKE3 of the file, as `run` prints it).
//! build = <64 hex digits>
//!
//! # Renderer port 2 (VICTORY_PORT_DESCRIPTORS=1). One line for each store of a
//! # 32-byte descriptor record (`vmovups [base+index], ymm0`: VEX, no
//! # displacement): the store's offset from the start of the match, its length
//! # in bytes (5 to 16) and the pattern. The pattern must run on to an
//! # instruction that sets the flags before they are read, as the patch does not
//! # keep them.
//! store = 10 5 : 81 E1 ?? ?? 00 00 C5 FC ...
//!
//! # The frame rate (VICTORY_FRAME_INTERVAL_MS). `frame.read`: a load of the
//! # frame rate setting (`mov eax, [reg+disp32]`, `<length>` bytes at the offset)
//! # becomes `mov eax, rate` and nops. `frame.step`: a check of the setting
//! # (7 bytes at the offset) becomes `mov eax, rate ; jmp` `<jump>` bytes on
//! # from its end.
//! frame.read = <offset> <length> : <pattern>
//! frame.step = <offset> <jump> : <pattern>
//!
//! # The output size and the render scale (VICTORY_OUTPUT, VICTORY_RENDER_SCALE,
//! # VICTORY_FFX_SCALE, VICTORY_UPSCALER). Each `<name>` is a pattern whose
//! # match is changed at its start.
//! resolution.modes = 1280x720 1920x1080 ...      # the sizes the title has a mode for
//! resolution.output = <pattern>
//! resolution.output.code = <template>
//! resolution.pool = <pattern>
//! resolution.pool.code = <template>
//! resolution.pool.unless = 3840x2160             # the output size that leaves it as it is
//! resolution.own = <pattern>
//! resolution.own.code = <template>
//! # A function that loads a scale from two places (`vmovss xmm, [rip+disp32]`,
//! # 8 bytes each, at these offsets from the start of the match); both are
//! # pointed at 4 bytes of the function's own padding (`slot`) that hold the new
//! # scale.
//! resolution.ffx = <pattern>
//! resolution.ffx.loads = 0x28 0x32
//! resolution.ffx.slot = 0x64
//! # A function that picks an upscaler: becomes `mov eax, kind ; ret`.
//! resolution.picker = <pattern>
//!
//! # Title data that VICTORY_TIMER_LOG=1 (diagnostics) logs once a second:
//! # a name, `u8`, `u32` or `f32`, and the data's offset in the image.
//! log = <name> <u8|u32|f32> <offset>
//! ```
//!
//! A *template* is the bytes that go in place of a match: hex bytes, and
//! `{width}`, `{height}` (4 bytes, the output size asked for), `{scale}` (an
//! `f32`, the scale asked for) and `{rel:<offset>}` (a `rel32` from the end of
//! its own 4 bytes to that offset in the image). It may not be longer than its
//! pattern.
//!
//! The idea (patterns found at launch, written only when found exactly once) is
//! Marvisak/VIctory pull request 1793's (GPL-3.0); the format is this project's
//! own.

use anyhow::{Context, Result, bail, ensure};
use victory_pe::SectionInfo;

/// A section that holds code.
const EXECUTE: u32 = 0x2000_0000;

/// Hex bytes, `None` for `??`.
pub type Pattern = Vec<Option<u8>>;

/// The executable sections of an image: the offset of each in the image and
/// its bytes.
pub type Code<'a> = [(usize, &'a [u8])];

/// A place in the code and the length of what is changed there: `<offset>
/// <length> : <pattern>`.
#[derive(Debug, Clone, PartialEq)]
pub struct Place {
    pub pattern: Pattern,
    /// The offset of the change from the start of the match.
    pub at: usize,
    pub len: usize,
}

/// The check the timers' step is made of: `<offset> <jump> : <pattern>`.
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub pattern: Pattern,
    pub at: usize,
    /// Bytes from the end of the 7 that are written to where the code goes on.
    pub jump: u8,
}

/// A pattern and the bytes that go over its start.
#[derive(Debug, Clone, PartialEq)]
pub struct Edit {
    pub pattern: Pattern,
    pub code: Template,
}

/// The back buffer pool check: an edit, left out for the output size `unless`.
#[derive(Debug, Clone, PartialEq)]
pub struct Pool {
    pub edit: Edit,
    pub unless: Option<(u32, u32)>,
}

/// A function that loads a scale in two places.
#[derive(Debug, Clone, PartialEq)]
pub struct Ffx {
    pub pattern: Pattern,
    pub loads: Vec<usize>,
    pub slot: usize,
}

/// What a logged datum is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    U8,
    U32,
    F32,
}

/// A datum of the title's that `VICTORY_TIMER_LOG` shows.
#[derive(Debug, Clone, PartialEq)]
pub struct Logged {
    pub name: String,
    pub kind: Kind,
    pub at: usize,
}

/// A parsed profile.
#[derive(Debug, Default, PartialEq)]
pub struct Profile {
    pub build: Option<String>,
    pub stores: Vec<Place>,
    pub frame_reads: Vec<Place>,
    pub frame_steps: Vec<Step>,
    pub modes: Vec<(u32, u32)>,
    pub output: Option<Edit>,
    pub pool: Option<Pool>,
    pub own_scale: Option<Edit>,
    pub ffx: Option<Ffx>,
    pub picker: Option<Pattern>,
    pub logged: Vec<Logged>,
}

/// One part of a template.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Part {
    Byte(u8),
    Width,
    Height,
    Scale,
    Rel(usize),
}

/// The bytes that go in place of a match.
#[derive(Debug, Clone, PartialEq)]
pub struct Template(Vec<Part>);

/// What the placeholders of a template stand for.
#[derive(Debug, Default, Clone, Copy)]
pub struct Values {
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub scale: Option<f32>,
}

fn number(text: &str) -> Result<usize> {
    let text = text.trim();
    match text.strip_prefix("0x") {
        Some(hex) => usize::from_str_radix(hex, 16),
        None => text.parse(),
    }
    .with_context(|| format!("not a number: {text}"))
}

/// `<w>x<h>`.
pub fn parse_size(text: &str) -> Option<(u32, u32)> {
    let (w, h) = text.trim().split_once(['x', 'X'])?;
    Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
}

/// A pattern of hex bytes and `??` (any byte), separated by spaces.
pub fn parse_pattern(text: &str) -> Result<Pattern> {
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

impl Template {
    /// Hex bytes and the placeholders `{width}`, `{height}`, `{scale}`,
    /// `{rel:<offset>}`, separated by spaces.
    pub fn parse(text: &str) -> Result<Self> {
        let parts = text
            .split_whitespace()
            .map(|word| {
                Ok(match word {
                    "{width}" => Part::Width,
                    "{height}" => Part::Height,
                    "{scale}" => Part::Scale,
                    _ => match word.strip_prefix("{rel:").and_then(|w| w.strip_suffix('}')) {
                        Some(to) => Part::Rel(number(to)?),
                        None => Part::Byte(
                            u8::from_str_radix(word, 16)
                                .with_context(|| format!("not a hex byte or a placeholder: {word}"))?,
                        ),
                    },
                })
            })
            .collect::<Result<Vec<_>>>()?;
        ensure!(!parts.is_empty(), "a template needs bytes");
        Ok(Self(parts))
    }

    /// How many bytes it makes.
    pub fn len(&self) -> usize {
        self.0
            .iter()
            .map(|p| if matches!(p, Part::Byte(_)) { 1 } else { 4 })
            .sum()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The bytes for a change at the offset `at` of the image.
    pub fn encode(&self, at: usize, values: &Values) -> Result<Vec<u8>, String> {
        let mut out = Vec::with_capacity(self.len());
        for part in &self.0 {
            match *part {
                Part::Byte(b) => out.push(b),
                Part::Width => out.extend(values.width.ok_or("it needs the output width")?.to_le_bytes()),
                Part::Height => out.extend(values.height.ok_or("it needs the output height")?.to_le_bytes()),
                Part::Scale => out.extend(values.scale.ok_or("it needs the scale")?.to_le_bytes()),
                Part::Rel(to) => {
                    let next = (at + out.len() + 4) as i64;
                    let rel = i32::try_from(to as i64 - next).map_err(|_| "a target is out of reach of a rel32")?;
                    out.extend(rel.to_le_bytes());
                }
            }
        }
        Ok(out)
    }
}

/// `<offset> <length> : <pattern>`.
fn parse_place(value: &str) -> Result<Place> {
    let (numbers, pattern) = value
        .split_once(':')
        .context("expected `<offset> <length> : <pattern>`")?;
    let mut numbers = numbers.split_whitespace().map(number);
    let at = numbers.next().context("it needs an offset")??;
    let len = numbers.next().context("it needs a length")??;
    ensure!(
        numbers.next().is_none(),
        "it takes an offset and a length, then : and the pattern"
    );
    let pattern = parse_pattern(pattern)?;
    ensure!(
        (5..=16).contains(&len),
        "the change is {len} bytes: a jump takes 5, an instruction at most 15"
    );
    ensure!(
        pattern
            .get(at..at + len)
            .is_some_and(|bytes| bytes.iter().all(Option::is_some)),
        "the change (offset {at}, {len} bytes) must lie on fixed bytes of the pattern"
    );
    Ok(Place { pattern, at, len })
}

/// `<offset> <jump> : <pattern>`: 7 bytes at the offset.
fn parse_step(value: &str) -> Result<Step> {
    let (numbers, pattern) = value
        .split_once(':')
        .context("expected `<offset> <jump> : <pattern>`")?;
    let mut numbers = numbers.split_whitespace().map(number);
    let at = numbers.next().context("it needs an offset")??;
    let jump = numbers.next().context("it needs a jump")??;
    ensure!(
        numbers.next().is_none(),
        "it takes an offset and a jump, then : and the pattern"
    );
    let jump = u8::try_from(jump).ok().filter(|j| *j < 0x80).context("the jump is 0 to 127")?;
    let pattern = parse_pattern(pattern)?;
    ensure!(
        pattern
            .get(at..at + 7)
            .is_some_and(|bytes| bytes.iter().all(Option::is_some)),
        "the 7 bytes at offset {at} must lie on fixed bytes of the pattern"
    );
    Ok(Step { pattern, at, jump })
}

fn parse_log(value: &str) -> Result<Logged> {
    let mut words = value.split_whitespace();
    let name = words.next().context("it needs a name")?.to_owned();
    let kind = match words.next().context("it needs u8, u32 or f32")? {
        "u8" => Kind::U8,
        "u32" => Kind::U32,
        "f32" => Kind::F32,
        other => bail!("not u8, u32 or f32: {other}"),
    };
    let at = number(words.next().context("it needs an offset")?)?;
    ensure!(words.next().is_none(), "it takes a name, a type and an offset");
    Ok(Logged { name, kind, at })
}

/// A key that may be given once.
fn once<T>(slot: &mut Option<T>, key: &str, value: T) -> Result<()> {
    ensure!(slot.replace(value).is_none(), "{key} is given twice");
    Ok(())
}

impl Profile {
    pub fn parse(text: &str) -> Result<Self> {
        let mut p = Self::default();
        let (mut output, mut output_code) = (None, None);
        let (mut pool, mut pool_code, mut pool_unless) = (None, None, None);
        let (mut own, mut own_code) = (None, None);
        let (mut ffx, mut loads, mut slot) = (None, None, None);
        for (n, line) in text.lines().enumerate() {
            let line = line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let at = || format!("profile line {}", n + 1);
            let (key, value) = line
                .split_once('=')
                .with_context(|| format!("{}: expected key = value", at()))?;
            let (key, value) = (key.trim(), value.trim());
            let done: Result<()> = (|| {
                match key {
                    "build" => {
                        ensure!(
                            value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit()),
                            "build is 64 hex digits"
                        );
                        once(&mut p.build, key, value.to_ascii_lowercase())?;
                    }
                    "store" => p.stores.push(parse_place(value)?),
                    "frame.read" => p.frame_reads.push(parse_place(value)?),
                    "frame.step" => p.frame_steps.push(parse_step(value)?),
                    "resolution.modes" => {
                        ensure!(p.modes.is_empty(), "{key} is given twice");
                        p.modes = value
                            .split_whitespace()
                            .map(|s| parse_size(s).with_context(|| format!("not <w>x<h>: {s}")))
                            .collect::<Result<_>>()?;
                        ensure!(!p.modes.is_empty(), "{key} needs sizes");
                    }
                    "resolution.output" => once(&mut output, key, parse_pattern(value)?)?,
                    "resolution.output.code" => once(&mut output_code, key, Template::parse(value)?)?,
                    "resolution.pool" => once(&mut pool, key, parse_pattern(value)?)?,
                    "resolution.pool.code" => once(&mut pool_code, key, Template::parse(value)?)?,
                    "resolution.pool.unless" => {
                        once(&mut pool_unless, key, parse_size(value).context("expected <w>x<h>")?)?;
                    }
                    "resolution.own" => once(&mut own, key, parse_pattern(value)?)?,
                    "resolution.own.code" => once(&mut own_code, key, Template::parse(value)?)?,
                    "resolution.ffx" => once(&mut ffx, key, parse_pattern(value)?)?,
                    "resolution.ffx.loads" => {
                        let list = value.split_whitespace().map(number).collect::<Result<Vec<_>>>()?;
                        ensure!(!list.is_empty(), "{key} needs offsets");
                        once(&mut loads, key, list)?;
                    }
                    "resolution.ffx.slot" => once(&mut slot, key, number(value)?)?,
                    "resolution.picker" => once(&mut p.picker, key, parse_pattern(value)?)?,
                    "log" => p.logged.push(parse_log(value)?),
                    other => bail!("unknown key {other}"),
                }
                Ok(())
            })();
            done.with_context(at)?;
        }
        p.output = edit("resolution.output", output, output_code)?;
        p.pool = edit("resolution.pool", pool, pool_code)?.map(|edit| Pool { edit, unless: pool_unless });
        ensure!(
            p.pool.is_some() || pool_unless.is_none(),
            "resolution.pool.unless without resolution.pool"
        );
        p.own_scale = edit("resolution.own", own, own_code)?;
        p.ffx = match (ffx, loads, slot) {
            (None, None, None) => None,
            (Some(pattern), Some(loads), Some(slot)) => {
                // Each load is `vmovss xmm, [rip+disp32]`: 8 bytes, the displacement last.
                ensure!(
                    loads.iter().all(|l| l + 8 <= pattern.len()) && slot + 4 <= pattern.len(),
                    "resolution.ffx.loads and .slot must lie inside the pattern"
                );
                Some(Ffx { pattern, loads, slot })
            }
            _ => bail!("resolution.ffx needs resolution.ffx.loads and resolution.ffx.slot, and the other way round"),
        };
        for (name, template) in [("output", &p.output), ("own", &p.own_scale)] {
            if let Some(e) = template {
                ensure!(
                    e.code.len() <= e.pattern.len(),
                    "resolution.{name}.code is longer than its pattern"
                );
            }
        }
        if let Some(pool) = &p.pool {
            ensure!(
                pool.edit.code.len() <= pool.edit.pattern.len(),
                "resolution.pool.code is longer than its pattern"
            );
        }
        Ok(p)
    }

    /// Whether the profile has nothing in it that does anything.
    pub fn is_empty(&self) -> bool {
        *self == Self { build: self.build.clone(), ..Self::default() }
    }

    /// Whether this profile is for the build of the executable with this
    /// BLAKE3 (lowercase hex): one that names no build is for any.
    pub fn is_for(&self, build: &str) -> bool {
        self.build.as_deref().is_none_or(|b| b.eq_ignore_ascii_case(build))
    }
}

/// A pattern and its code, both or neither.
fn edit(key: &str, pattern: Option<Pattern>, code: Option<Template>) -> Result<Option<Edit>> {
    match (pattern, code) {
        (None, None) => Ok(None),
        (Some(pattern), Some(code)) => Ok(Some(Edit { pattern, code })),
        (Some(_), None) => bail!("{key} needs {key}.code"),
        (None, Some(_)) => bail!("{key}.code needs {key}"),
    }
}

/// Reads and parses the profile at `path`.
pub fn load(path: &std::path::Path) -> Result<Profile> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading the title profile {}", path.display()))?;
    Profile::parse(&text).with_context(|| format!("in the title profile {}", path.display()))
}

/// Every place in `code` where `pattern` matches, as offsets in `code`.
pub fn matches(code: &[u8], pattern: &[Option<u8>]) -> Vec<usize> {
    let mut found = Vec::new();
    let Some(k) = pattern.iter().position(Option::is_some) else {
        return found;
    };
    let Some(first) = pattern[k] else {
        return found;
    };
    let Some(last) = code.len().checked_sub(pattern.len()) else {
        return found;
    };
    // `hay[i]` is `code[i + k]`: the byte a match starting at `i` has first.
    let hay = &code[k..=last + k];
    let mut from = 0;
    while let Some(p) = hay[from..].iter().position(|&b| b == first) {
        let start = from + p;
        if pattern
            .iter()
            .zip(&code[start..])
            .all(|(want, &have)| want.is_none_or(|w| w == have))
        {
            found.push(start);
        }
        from = start + 1;
    }
    found
}

/// The offset in the image where `pattern` matches in `code`, if it matches
/// exactly once; or why not.
pub fn find_once(code: &Code, pattern: &[Option<u8>]) -> Result<usize, String> {
    let mut hits = code
        .iter()
        .flat_map(|&(offset, bytes)| matches(bytes, pattern).into_iter().map(move |m| offset + m));
    match (hits.next(), hits.next()) {
        (Some(at), None) => Ok(at),
        (None, _) => Err("its code is not in this build of the title".into()),
        (Some(_), Some(_)) => Err(format!("its pattern matches {} places in this build", 2 + hits.count())),
    }
}

/// The executable sections of the image of `size` bytes mapped at `base`, as
/// [`find_once`] takes them.
///
/// # Safety
/// The image is mapped at `base`, whole to `size`, and its code readable and
/// staying so for as long as the slices are used.
pub unsafe fn code_of<'a>(base: usize, size: usize, sections: &[SectionInfo]) -> Vec<(usize, &'a [u8])> {
    sections
        .iter()
        .filter(|s| s.characteristics & EXECUTE != 0)
        .filter_map(|s| {
            let len = match s.virtual_size {
                0 => s.file_size,
                vs => vs.min(s.file_size),
            } as usize;
            let at = base.checked_add(s.rva as usize)?;
            (s.rva as usize).checked_add(len).filter(|&end| end <= size)?;
            // SAFETY: by the caller; the section lies inside the image.
            Some((s.rva as usize, unsafe { std::slice::from_raw_parts(at as *const u8, len) }))
        })
        .collect()
}

/// A place found in the image, or why not: the line a note about it needs.
pub fn note(what: &str, n: usize, why: &str) -> String {
    format!("{what} {}: not patched, {why}", n + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Made-up bytes, not any title's.
    const PLACE_A: &str = "A1 A2 ?? ?? A3 A4 10 20 30 40 50 B1 B2 ?? ?? C1";
    const PLACE_B: &str = "D1 ?? D2 D3 ?? ?? 61 62 63 64 65 66 E1 E2 ?? E3 E4";

    fn instance(pattern: &str, fill: u8) -> Vec<u8> {
        parse_pattern(pattern)
            .unwrap()
            .into_iter()
            .map(|p| p.unwrap_or(fill))
            .collect()
    }

    fn error_of(text: &str) -> String {
        format!("{:#}", Profile::parse(text).unwrap_err())
    }

    #[test]
    fn an_empty_profile_does_nothing() {
        let profile = Profile::parse("# nothing\n\n").unwrap();
        assert!(profile.is_empty());
        assert!(profile.is_for("00"), "no build named: for any");
        let named = Profile::parse(&format!("build = {}", "AB".repeat(32))).unwrap();
        assert!(named.is_empty());
        assert!(named.is_for(&"ab".repeat(32)) && !named.is_for(&"cd".repeat(32)));
    }

    #[test]
    fn a_profile_with_every_key_parses() {
        let text = format!(
            "# made up\n\
             build = {}\n\
             store = 6 5 : {PLACE_A}   # first\n\
             store = 6 6 : {PLACE_B}\n\
             frame.read = 0 6 : 11 22 33 44 55 66 ?? ??\n\
             frame.step = 1 13 : ?? 11 22 33 44 55 66 77 ??\n\
             resolution.modes = 640x360 1280X720\n\
             resolution.output = 01 02 03 04 05 06 07 08 09 0A 0B 0C 0D 0E 0F 10\n\
             resolution.output.code = B8 {{width}} BF {{height}} E8 {{rel:0x100}}\n\
             resolution.pool = 11 22 33 44 55\n\
             resolution.pool.code = 11 22 33 EB 01\n\
             resolution.pool.unless = 1280x720\n\
             resolution.own = 01 02 03 04 05 06 07 08\n\
             resolution.own.code = 01 02 03 04 {{scale}}\n\
             resolution.ffx = 01 02 03 04 05 06 07 08 09 0A 0B 0C 0D 0E 0F 10 11 12 13 14\n\
             resolution.ffx.loads = 0x2 4\n\
             resolution.ffx.slot = 0x10\n\
             resolution.picker = 48 83 ?? 28\n\
             log = frame u32 0x1000\n\
             log = flag u8 4096\n\
             log = step f32 0x2000\n",
            "ab".repeat(32)
        );
        let p = Profile::parse(&text).unwrap();
        assert_eq!(p.stores.len(), 2);
        assert_eq!((p.stores[0].at, p.stores[0].len), (6, 5));
        assert_eq!(p.stores[1].pattern.len(), 17);
        assert_eq!(p.frame_reads.len(), 1);
        assert_eq!((p.frame_steps[0].at, p.frame_steps[0].jump), (1, 13));
        assert_eq!(p.modes, [(640, 360), (1280, 720)]);
        assert_eq!(p.output.as_ref().unwrap().code.len(), 1 + 4 + 1 + 4 + 1 + 4);
        assert_eq!(p.pool.as_ref().unwrap().unless, Some((1280, 720)));
        assert_eq!(p.own_scale.as_ref().unwrap().code.len(), 8);
        assert_eq!(p.ffx, Some(Ffx { pattern: parse_pattern("01 02 03 04 05 06 07 08 09 0A 0B 0C 0D 0E 0F 10 11 12 13 14").unwrap(), loads: vec![2, 4], slot: 16 }));
        assert_eq!(p.picker.as_ref().unwrap().len(), 4);
        assert_eq!(
            p.logged,
            [
                Logged { name: "frame".into(), kind: Kind::U32, at: 0x1000 },
                Logged { name: "flag".into(), kind: Kind::U8, at: 0x1000 },
                Logged { name: "step".into(), kind: Kind::F32, at: 0x2000 },
            ]
        );
        assert!(!p.is_empty());
    }

    #[test]
    fn a_bad_profile_says_what_is_wrong_and_where() {
        for (bad, why) in [
            ("colour = 1", "unknown key"),
            ("store", "expected key = value"),
            ("build = 12", "64 hex digits"),
            ("store = 0 5 01 02 03 04 05", "expected `<offset> <length> : <pattern>`"),
            ("store = 0 : 01 02 03 04 05", "needs a length"),
            ("store = x 5 : 01 02 03 04 05", "not a number"),
            ("store = 0 5 1 : 01 02 03 04 05", "takes an offset and a length"),
            ("store = 0 4 : 01 02 03 04 05", "a jump takes 5"),
            ("store = 0 17 : 01 02 03 04 05 06 07 08 09 0A 0B 0C 0D 0E 0F 10 11", "a jump takes 5"),
            ("store = 0 5 : 01 02 03 04", "fixed bytes of the pattern"),
            ("store = 1 5 : 01 02 03 04 05", "fixed bytes of the pattern"),
            ("store = 0 5 : 01 02 ?? 04 05", "fixed bytes of the pattern"),
            ("store = 0 5 : ?? ?? ?? ?? ??", "at least one fixed byte"),
            ("store = 0 5 : 01 02 03 04 zz", "not a hex byte"),
            ("frame.step = 0 128 : 01 02 03 04 05 06 07", "0 to 127"),
            ("frame.step = 0 5 : 01 02 03 04 05 06", "fixed bytes of the pattern"),
            ("frame.read = 0 6", "expected"),
            ("resolution.modes = 1280", "not <w>x<h>"),
            ("resolution.modes =", "needs sizes"),
            ("resolution.output = 01 02", "resolution.output needs resolution.output.code"),
            ("resolution.output.code = 01", "resolution.output.code needs resolution.output"),
            ("resolution.output = 01 02\nresolution.output.code = 01 02 03", "longer than its pattern"),
            ("resolution.output = 01\nresolution.output = 02", "given twice"),
            ("resolution.output.code = {depth}", "not a hex byte or a placeholder"),
            ("resolution.pool.unless = 1x1", "without resolution.pool"),
            ("resolution.ffx = 01 02 03 04 05 06 07 08", "needs resolution.ffx.loads"),
            ("resolution.ffx.slot = 4", "needs resolution.ffx.loads"),
            (
                "resolution.ffx = 01 02 03 04 05 06 07 08 09 0A\nresolution.ffx.loads = 4\nresolution.ffx.slot = 4",
                "inside the pattern",
            ),
            ("log = a u16 4", "not u8, u32 or f32"),
            ("log = a u8", "needs an offset"),
        ] {
            let error = error_of(bad);
            assert!(error.contains(why), "{bad:?}: {error}");
        }
        assert!(error_of("\n\n store = 0 5 : 01").contains("profile line 3"));
        // Open bytes outside the change are fine.
        assert!(Profile::parse("store = 2 5 : ?? ?? 01 02 03 04 05 ??").is_ok());
    }

    #[test]
    fn a_template_makes_its_bytes() {
        let t = Template::parse("B8 {width} 89 {height} E8 {rel:0x200} {scale} 90").unwrap();
        assert_eq!(t.len(), 1 + 4 + 1 + 4 + 1 + 4 + 4 + 1);
        let v = Values { width: Some(1920), height: Some(1080), scale: Some(0.5) };
        let bytes = t.encode(0x1000, &v).unwrap();
        assert_eq!(bytes.len(), t.len());
        assert_eq!(bytes[0], 0xb8);
        assert_eq!(bytes[1..5], 1920u32.to_le_bytes());
        assert_eq!(bytes[6..10], 1080u32.to_le_bytes());
        // The rel32 is from the end of its own four bytes (at 0x1000 + 15) to 0x200.
        assert_eq!(i32::from_le_bytes(bytes[11..15].try_into().unwrap()), 0x200 - (0x1000 + 15));
        assert_eq!(bytes[15..19], 0.5f32.to_le_bytes());
        assert_eq!(bytes[19], 0x90);
        // A placeholder with nothing to stand for, a target out of reach.
        assert!(t.encode(0, &Values::default()).unwrap_err().contains("width"));
        let far = Template::parse("E8 {rel:0x1}").unwrap();
        assert!(far.encode(usize::MAX / 4, &Values::default()).unwrap_err().contains("out of reach"));
        assert!(Template::parse("").is_err());
    }

    #[test]
    fn a_pattern_is_found_exactly_once() {
        let (a, b) = (instance(PLACE_A, 0x11), instance(PLACE_B, 0x22));
        let pa = parse_pattern(PLACE_A).unwrap();
        let pb = parse_pattern(PLACE_B).unwrap();
        let mut text = vec![0x90u8; 0x30];
        text.extend(&a);
        text.extend([0xcc; 7]);
        let b_at = text.len();
        text.extend(&b);
        text.extend([0x90; 9]);
        assert_eq!(find_once(&[(0x4000, &text)], &pa), Ok(0x4000 + 0x30));
        assert_eq!(find_once(&[(0x4000, &text)], &pb), Ok(0x4000 + b_at));
        // Open bytes may be anything; twice is said and gives none, even in two sections.
        let mut more = text.clone();
        more.extend(instance(PLACE_A, 0x77));
        let err = find_once(&[(0x1000, &more)], &pa).unwrap_err();
        assert!(err.contains("matches 2 places"), "{err}");
        let err = find_once(&[(0x1000, &a), (0x9000, &a), (0x5000, &a)], &pa).unwrap_err();
        assert!(err.contains("matches 3 places"), "{err}");
        // Missing; a changed fixed byte; cut off at the end; shorter than the pattern.
        assert!(find_once(&[(0x1000, &b)], &pa).unwrap_err().contains("not in this build"));
        let mut changed = a.clone();
        changed[8] = 0x31;
        assert!(find_once(&[(0x1000, &changed)], &pa).is_err());
        assert!(find_once(&[(0x1000, &a[..a.len() - 1])], &pa).is_err());
        assert!(find_once(&[(0x1000, &[])], &pa).is_err());
        // A match at the very start and at the very end.
        assert_eq!(matches(&a, &pa), [0]);
        let mut both = a.clone();
        both.extend(&a);
        assert_eq!(matches(&both, &pa), [0, a.len()]);
        // A pattern that starts with open bytes.
        let open = parse_pattern("?? ?? 61 62").unwrap();
        assert_eq!(matches(&[0, 1, 0x61, 0x62, 0x61, 0x62], &open), [0, 2]);
    }

    #[test]
    fn the_executable_sections_are_the_ones_read() {
        let section = |rva: u32, len: u32, characteristics: u32| SectionInfo {
            name: ".x".into(),
            rva,
            virtual_size: len,
            file_offset: 0,
            file_size: len,
            characteristics,
        };
        let image = vec![0x5au8; 0x400];
        let base = image.as_ptr() as usize;
        let sections = [
            section(0x100, 0x80, EXECUTE | 0x4000_0000),
            section(0x200, 0x80, 0x4000_0000),
            section(0x3c0, 0x80, EXECUTE),
            section(0xffff_0000, 0x10, EXECUTE),
        ];
        // SAFETY: the image is a vector of ours, alive for the whole test.
        let code = unsafe { code_of(base, image.len(), &sections) };
        assert_eq!(code.len(), 1, "data, and what lies outside the image, are not read");
        assert_eq!((code[0].0, code[0].1.len()), (0x100, 0x80));
    }

    #[test]
    fn a_file_is_read_and_a_bad_one_is_named() {
        let dir = tempfile::tempdir().unwrap();
        let good = dir.path().join("profile.txt");
        std::fs::write(&good, "store = 0 5 : 01 02 03 04 05\n").unwrap();
        assert_eq!(load(&good).unwrap().stores.len(), 1);
        let bad = dir.path().join("bad.txt");
        std::fs::write(&bad, "oops\n").unwrap();
        let error = format!("{:#}", load(&bad).unwrap_err());
        assert!(error.contains("bad.txt") && error.contains("profile line 1"), "{error}");
        let error = format!("{:#}", load(&dir.path().join("none.txt")).unwrap_err());
        assert!(error.contains("none.txt"), "{error}");
    }

    /// The user's own profile against the user's own title: the executable
    /// (`VICTORY_TITLE_IMAGE`) is read, nothing runs or is written. Skipped
    /// unless both it and `VICTORY_TITLE_PROFILE` are set.
    #[test]
    fn a_users_profile_finds_its_places_in_a_users_title() {
        let (Some(image), Some(profile)) = (
            std::env::var_os("VICTORY_TITLE_IMAGE"),
            std::env::var_os("VICTORY_TITLE_PROFILE"),
        ) else {
            return;
        };
        let profile = load(std::path::Path::new(&profile)).unwrap();
        let prepared = crate::prepare(&std::fs::read(image).unwrap()).unwrap();
        println!("build {}", prepared.original_hash.to_hex());
        assert!(profile.is_for(prepared.original_hash.to_hex().as_str()), "a profile of another build");
        let sections: Vec<(usize, &[u8])> = prepared
            .info
            .sections
            .iter()
            .filter(|s| s.characteristics & EXECUTE != 0)
            .map(|s| {
                let len = match s.virtual_size {
                    0 => s.file_size,
                    vs => vs.min(s.file_size),
                };
                let from = s.file_offset as usize;
                // The address of a match is its image offset.
                (s.rva as usize, &prepared.bytes[from..from + len as usize])
            })
            .collect();
        let mut patterns: Vec<(String, &Pattern)> = Vec::new();
        patterns.extend(profile.stores.iter().map(|p| ("store".into(), &p.pattern)));
        patterns.extend(profile.frame_reads.iter().map(|p| ("frame.read".into(), &p.pattern)));
        patterns.extend(profile.frame_steps.iter().map(|p| ("frame.step".into(), &p.pattern)));
        patterns.extend(profile.output.iter().map(|e| ("resolution.output".into(), &e.pattern)));
        patterns.extend(profile.pool.iter().map(|p| ("resolution.pool".into(), &p.edit.pattern)));
        patterns.extend(profile.own_scale.iter().map(|e| ("resolution.own".into(), &e.pattern)));
        patterns.extend(profile.ffx.iter().map(|f| ("resolution.ffx".into(), &f.pattern)));
        patterns.extend(profile.picker.iter().map(|p| ("resolution.picker".into(), p)));
        assert!(!patterns.is_empty());
        for (name, pattern) in &patterns {
            let found = find_once(&sections, pattern);
            println!("{name}: {found:x?}");
            assert!(found.is_ok(), "{name}: {found:?}");
        }
    }
}
