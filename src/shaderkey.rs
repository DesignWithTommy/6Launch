//! Rebuilt runtimes keep their shader cache keys.
//!
//! Most VIctory forks key every cached shader on the bytes of victory.exe
//! (victory-gpu shader_cache.rs build()), so any rebuild (a patch, a new
//! folder, a new toolchain) translated every shader again. For the build,
//! i95 makes that read take `neonbridge-key.bin` beside the exe when
//! it is there: a copy of the exe the cache was made with. So a rebuilt exe
//! makes exactly the old keys. The first build of a runtime with no earlier
//! exe keys on itself. The edit is put back after the build, like the
//! patches. NEONBRIDGE_SHADER_KEY_FIX=0 turns it off.

use std::fs;
use std::path::{Path, PathBuf};

/// The exe read, however each fork words the statement around it.
const LINE: &str = "std::env::current_exe().and_then(std::fs::read)";
const WITH: &str = "std::env::current_exe().and_then(|e| std::fs::read(e.with_file_name(\"neonbridge-key.bin\")).or_else(|_| std::fs::read(&e)))";
/// Belobug's form (it may borrow another build's exe first).
const LINE2: &str = "other.map_or_else(std::env::current_exe, Ok).and_then(std::fs::read)";
const WITH2: &str = "other.map_or_else(std::env::current_exe, Ok).and_then(|e| std::fs::read(e.with_file_name(\"neonbridge-key.bin\")).or_else(|_| std::fs::read(&e)))";
pub const KEY_BIN: &str = "neonbridge-key.bin";

pub fn on() -> bool {
    // Off by default: NEONBRIDGE_SHADER_KEY_FIX=1 turns it on.
    std::env::var("NEONBRIDGE_SHADER_KEY_FIX").is_ok_and(|v| v.trim() == "1")
}

/// The exe-keyed shader_cache.rs files under a runtime's tree.
pub fn files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk(root, 0, &mut out);
    out
}

fn walk(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    if depth > 6 {
        return;
    }
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let n = e.file_name();
        let n = n.to_string_lossy();
        if p.is_dir() {
            if !matches!(n.as_ref(), "target" | ".git" | "node_modules" | "docs") {
                walk(&p, depth + 1, out);
            }
        } else if n == "shader_cache.rs" && p.parent().and_then(Path::parent).is_some_and(|g| g.ends_with("victory-gpu")) {
            if fs::read_to_string(&p).is_ok_and(|t| t.contains(LINE) || t.contains(LINE2)) {
                out.push(p);
            }
        }
    }
}

/// The tree's exe must be (re)built with the key read: it is exe-keyed and
/// has no key file yet.
pub fn needs_build(root: &Path, exe: &Path) -> bool {
    on() && !exe.with_file_name(KEY_BIN).is_file() && !files(root).is_empty()
}

/// Swaps the read in; returns the originals to put back.
pub fn apply(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut kept = Vec::new();
    if !on() {
        return kept;
    }
    for f in files(root) {
        let Ok(orig) = fs::read(&f) else { continue };
        let text = String::from_utf8_lossy(&orig).replace(LINE, WITH).replace(LINE2, WITH2);
        if fs::write(&f, text).is_ok() {
            println!("[launch] shader keys kept across rebuilds ({})", f.display());
            kept.push((f, orig));
        }
    }
    kept
}

pub fn revert(kept: &[(PathBuf, Vec<u8>)]) {
    for (f, orig) in kept {
        let _ = fs::write(f, orig);
    }
}

/// After a build: the key file beside the new exe is the exe the cache was
/// made with (`old`, the earlier build), else the new exe itself.
pub fn seed(exe: &Path, old: Option<&Path>) {
    let bin = exe.with_file_name(KEY_BIN);
    if bin.is_file() {
        return;
    }
    let from = old.filter(|o| o.is_file() && *o != exe).unwrap_or(exe);
    match fs::copy(from, &bin) {
        Ok(_) => println!("[launch] shader key: {} (keys of {})", bin.display(), from.display()),
        Err(e) => println!("[launch] shader key file not written ({e})"),
    }
}
