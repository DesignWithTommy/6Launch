//! Shader cache keys that follow the translator, not the exe.
//!
//! Most VIctory forks key every cached shader on the bytes of victory.exe
//! (victory-gpu shader_cache.rs build()), so any rebuild (a patch, a new
//! folder, a new toolchain) translated every shader again. For the build,
//! Neonbridge swaps that one line for a key the launcher computes from the
//! tree's victory-shader sources and shader_cache.rs, passed in as
//! NEONBRIDGE_SHADER_KEY; the switches the runtime appends stay. The line is
//! put back after the build, like the patches. NEONBRIDGE_SHADER_KEY_FIX=0 off.

use std::fs;
use std::path::{Path, PathBuf};

/// The exe read, however each fork words the statement around it.
const LINE: &str = "std::env::current_exe().and_then(std::fs::read)";
const WITH: &str = "(match option_env!(\"NEONBRIDGE_SHADER_KEY\") { Some(k) => Ok(k.as_bytes().to_vec()), None => std::env::current_exe().and_then(std::fs::read) })";

pub fn on() -> bool {
    std::env::var("NEONBRIDGE_SHADER_KEY_FIX").map_or(true, |v| v.trim() != "0")
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
            if fs::read_to_string(&p).is_ok_and(|t| t.contains(LINE)) {
                out.push(p);
            }
        }
    }
}

/// sha256 of the tree's crates/victory-shader (every file, by relative path)
/// and the original shader_cache.rs.
pub fn key(cache_rs: &Path) -> Option<String> {
    use sha2::{Digest, Sha256};
    let tree = cache_rs.parent()?.parent()?.parent()?.parent()?;
    let shader = tree.join("crates").join("victory-shader");
    let mut all = Vec::new();
    collect(&shader, &mut all);
    all.sort();
    let mut h = Sha256::new();
    h.update(b"neonbridge shader key v1\0");
    for f in &all {
        h.update(f.strip_prefix(&shader).ok()?.to_string_lossy().replace('\\', "/").as_bytes());
        h.update([0]);
        h.update(fs::read(f).ok()?);
    }
    h.update(fs::read(cache_rs).ok()?);
    Some(format!("{:x}", h.finalize()))
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            if e.file_name() != "target" {
                collect(&p, out);
            }
        } else {
            out.push(p);
        }
    }
}

/// Swaps the line in; returns the originals to put back and the key.
pub fn apply(root: &Path) -> (Vec<(PathBuf, Vec<u8>)>, Option<String>) {
    let mut kept = Vec::new();
    let mut k = None;
    if !on() {
        return (kept, k);
    }
    for f in files(root) {
        let Ok(orig) = fs::read(&f) else { continue };
        let Some(key) = key(&f) else { continue };
        let text = String::from_utf8_lossy(&orig).replace(LINE, WITH);
        if fs::write(&f, text).is_ok() {
            println!("[launch] shader keys follow the translator ({}), not the exe", f.display());
            kept.push((f, orig));
            k.get_or_insert(key);
        }
    }
    (kept, k)
}

pub fn revert(kept: &[(PathBuf, Vec<u8>)]) {
    for (f, orig) in kept {
        let _ = fs::write(f, orig);
    }
}
