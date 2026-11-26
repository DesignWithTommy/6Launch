//! The translator's identity for the shader cache (`shader_cache::build`):
//! a hash of the translator's sources (the whole `victory-shader` crate),
//! of how this crate encodes its translations, and of the lock file (the
//! versions of what they use). A rebuild that changes none of these keeps
//! the cache, which keying by our executable threw away at every build.

use std::path::{Path, PathBuf};

fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            files(&path, out);
        } else {
            out.push(path);
        }
    }
}

fn main() {
    let here = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("set by cargo"));
    let shader = here.join("../victory-shader");
    let mut inputs = vec![
        shader.join("Cargo.toml"),
        here.join("src/shader_cache.rs"),
        here.join("../../Cargo.lock"),
    ];
    files(&shader.join("src"), &mut inputs);
    // Sorted by path relative to the workspace, so the hash does not depend
    // on where the checkout is.
    let root = here.join("../..");
    let mut named: Vec<(String, PathBuf)> = inputs
        .into_iter()
        .map(|p| {
            let rel = p
                .strip_prefix(&root)
                .unwrap_or(&p)
                .to_string_lossy()
                .replace('\\', "/");
            (rel, p)
        })
        .collect();
    named.sort();
    let mut hasher = blake3::Hasher::new();
    for (rel, path) in &named {
        hasher.update(rel.as_bytes());
        hasher.update(&[0]);
        // Line endings do not change the translation.
        let text = std::fs::read(path).unwrap_or_default();
        let text: Vec<u8> = text.into_iter().filter(|&b| b != b'\r').collect();
        hasher.update(&(text.len() as u64).to_le_bytes());
        hasher.update(&text);
    }
    println!(
        "cargo:rustc-env=VICTORY_TRANSLATOR_HASH={}",
        hasher.finalize().to_hex()
    );
    println!("cargo:rerun-if-changed={}", shader.join("src").display());
    println!(
        "cargo:rerun-if-changed={}",
        shader.join("Cargo.toml").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        here.join("src/shader_cache.rs").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        root.join("Cargo.lock").display()
    );
}
