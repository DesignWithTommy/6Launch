//! Runtimes fetched on demand: a vendor/<name> that holds only its
//! NEONBRIDGE.toml, UPSTREAM and credits (no sources, no built exe) builds
//! from a cached copy of its upstream at the pinned commit, downloaded once
//! (after a yes) into %LOCALAPPDATA%\6ixpack\src\<name>-<commit>.
//! GitHub's own zip of the commit is used, so no git is needed.

use std::fs;
use std::path::{Path, PathBuf};

/// Where a runtime's sources live: vendor/<name> when it holds them (or a
/// packaged exe), else the source cache for its pinned commit.
/// Written by --package beside a shipped runtime's exe.
pub const PACKAGED: &str = "PACKAGED";

pub fn source(name: &str, vendor_dir: &Path, exes: &[&str]) -> PathBuf {
    // A tree or a packaged runtime. (An exe built in vendor/<name> before is
    // the key source for the rebuilt one: shaderkey.rs.)
    let _ = exes;
    if vendored(vendor_dir) || vendor_dir.join(PACKAGED).is_file() {
        return vendor_dir.to_path_buf();
    }
    match pin(vendor_dir) {
        Some(commit) => cache_dir(name, &commit),
        None => vendor_dir.to_path_buf(),
    }
}

/// The folder holds a runtime's sources.
pub fn has_sources(dir: &Path) -> bool {
    vendored(dir)
}

/// vendor/<name> holds a tree of its own.
fn vendored(dir: &Path) -> bool {
    ["Cargo.toml", "apps", "crates", "third_party", "src", "runtime"].iter().any(|p| has_source_file(&dir.join(p)))
}

/// `p` is a file, or a folder with a file outside any `target` folder: a
/// leftover build output alone (vendor/anyxboxx/third_party/victory/target,
/// 1.2 GB) is not a source tree, and counting it built in a folder with no
/// Cargo.toml ("manifest path ... does not exist").
fn has_source_file(p: &Path) -> bool {
    if p.is_file() {
        return true;
    }
    let Ok(rd) = fs::read_dir(p) else { return false };
    rd.flatten().any(|e| {
        let q = e.path();
        if q.is_dir() { q.file_name().is_some_and(|n| n != "target") && has_source_file(&q) } else { true }
    })
}

/// The full commit in vendor/<name>/UPSTREAM.
fn pin(dir: &Path) -> Option<String> {
    let text = fs::read_to_string(dir.join("UPSTREAM")).ok()?;
    let c = text.lines().next()?.trim().to_owned();
    (c.len() == 40 && c.chars().all(|c| c.is_ascii_hexdigit())).then_some(c)
}

fn cache_dir(name: &str, commit: &str) -> PathBuf {
    super::runtimes::pack_dir().join("src").join(format!("{name}-{}", &commit[..12]))
}

/// "owner/repo" from a github.com upstream URL.
fn repo(upstream: &str) -> Option<String> {
    let rest = upstream.trim().trim_end_matches('/').trim_end_matches(".git").split("github.com/").nth(1)?;
    let mut parts = rest.split('/');
    let (o, r) = (parts.next()?, parts.next()?);
    let ok = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c));
    (ok(o) && ok(r)).then(|| format!("{o}/{r}"))
}

/// Makes sure `root` (from source()) exists: downloads and unpacks the
/// pinned commit after asking. Older commits are kept as offline copies.
pub fn ensure(name: &str, vendor_dir: &Path, root: &Path, upstream: Option<&str>) -> Result<(), String> {
    if root == vendor_dir || root.join(".nbfetched").is_file() {
        return Ok(());
    }
    let commit = pin(vendor_dir).ok_or_else(|| format!("{name}: vendor/{name}/UPSTREAM has no commit"))?;
    // Bundled with i95: vendor/<name>/source holds the pinned commit's tree
    // (no download, works offline). Copied, so builds and exes stay in the
    // source cache and the shader keys and caches made there are kept.
    let bundled = vendor_dir.join(BUNDLED);
    if fs::read_to_string(bundled.join(BUNDLED_COMMIT)).is_ok_and(|c| c.trim() == commit) {
        crate::splash::status(0.05, &format!("Unpacking {name} (bundled)"));
        copy_tree(&bundled, root).map_err(|e| format!("{name}: copying the bundled source: {e}"))?;
        let _ = fs::remove_file(root.join(BUNDLED_COMMIT));
        fs::write(root.join(".nbfetched"), &commit).map_err(|e| e.to_string())?;
        println!("[fetch] {name}: bundled source {} ready", root.display());
        return Ok(());
    }
    let repo = upstream.and_then(repo).ok_or_else(|| format!("{name}: its toml's upstream is not a github.com repository"))?;
    let url = format!("https://codeload.github.com/{repo}/zip/{commit}");
    if !ask(&format!("Download {name}'s source from github.com/{repo} (commit {}) into {}?\n\nIt is downloaded once and kept; it is built on this PC.", &commit[..7], root.display())) {
        return Err(format!("{name}: download declined"));
    }
    let parent = root.parent().ok_or("no source folder")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let work = parent.join(format!(".{name}-{}-part", &commit[..12]));
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let zip = work.join("src.zip");
    crate::splash::status(0.05, &format!("Downloading {name}"));
    println!("[fetch] {name}: {url}");
    let ok = |c: &mut std::process::Command| c.status().is_ok_and(|s| s.success());
    if !ok(super::runtimes::command("curl").args(["-L", "--fail", "-sS", "-o"]).arg(&zip).arg(&url)) {
        return Err(format!("{name}: download failed ({url}); the repository may be gone"));
    }
    crate::splash::status(0.08, &format!("Unpacking {name}"));
    // Windows' own tar (bsdtar) reads zips.
    // Not PATH's tar: Git's GNU tar can't read zips.
    let tar = PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into())).join(r"System32\tar.exe");
    if !ok(super::runtimes::command(&tar).arg("-xf").arg(&zip).arg("-C").arg(&work)) {
        return Err(format!("{name}: unpacking {} failed", zip.display()));
    }
    let _ = fs::remove_file(&zip);
    // The zip holds one folder, <repo>-<commit>.
    let top = fs::read_dir(&work).map_err(|e| e.to_string())?.flatten().map(|e| e.path()).find(|p| p.is_dir()).ok_or_else(|| format!("{name}: the zip held no folder"))?;
    let _ = fs::remove_dir_all(root);
    fs::rename(&top, root).map_err(|e| format!("{name}: {e}"))?;
    let _ = fs::remove_dir_all(&work);
    fs::write(root.join(".nbfetched"), &commit).map_err(|e| e.to_string())?;
    // Older commits stay: they are the user's offline copies.
    println!("[fetch] {name}: {} ready", root.display());
    Ok(())
}

/// vendor/<name>/source: the bundled tree, with the commit it is in COMMIT.
pub const BUNDLED: &str = "source";
const BUNDLED_COMMIT: &str = "COMMIT";

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for e in fs::read_dir(from)? {
        let e = e?;
        let dst = to.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_tree(&e.path(), &dst)?;
        } else {
            fs::copy(e.path(), dst)?;
        }
    }
    Ok(())
}

fn ask(text: &str) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{IDYES, MB_DEFBUTTON2, MB_ICONQUESTION, MB_SETFOREGROUND, MB_TOPMOST, MB_YESNO, MessageBoxW};
    if std::env::var("NEONBRIDGE_FETCH").is_ok_and(|v| v.trim() == "1") {
        return true;
    }
    let wide = |t: &str| t.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
    let (text, title) = (wide(text), wide("i95"));
    // SAFETY: both strings are NUL-terminated and live across the call.
    unsafe { MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2 | MB_TOPMOST | MB_SETFOREGROUND) == IDYES }
}

#[cfg(test)]
mod tests {
    #[test]
    fn repo_from_upstream() {
        assert_eq!(super::repo("https://github.com/proneonn/Vertex").as_deref(), Some("proneonn/Vertex"));
        assert_eq!(super::repo("https://github.com/a/b.git/").as_deref(), Some("a/b"));
        assert_eq!(super::repo("https://example.com/a/b"), None);
    }
}

/// The runtime list on GitHub: every runtime's NEONBRIDGE.toml and pin in
/// one file, so a release picks up new runtimes and settings without a rebuild.
pub const CATALOG_URL: &str = "https://raw.githubusercontent.com/DesignWithTommy/i95-GTA6-PC-Launcher/main/catalog.txt";
const CATALOG_MARK: &str = "=== ";

/// `--catalog`: catalog.txt at the repo root from vendor/*/NEONBRIDGE.toml + UPSTREAM.
pub fn write_catalog(vendor: &Path, out: &Path) -> Result<usize, String> {
    let mut text = String::from("# i95 runtime catalog: `=== <name> <upstream commit>` then that runtime's NEONBRIDGE.toml. Made by i95 --catalog.\n");
    let mut names: Vec<_> = fs::read_dir(vendor).map_err(|e| e.to_string())?.flatten().map(|e| e.path()).collect();
    names.sort();
    let mut n = 0;
    for dir in names {
        let (Ok(toml), Some(commit)) = (fs::read_to_string(dir.join("NEONBRIDGE.toml")), pin(&dir)) else { continue };
        let name = dir.file_name().unwrap_or_default().to_string_lossy().into_owned();
        text.push_str(&format!("{CATALOG_MARK}{name} {commit}\n{}\n", toml.replace("\r\n", "\n").trim_end()));
        n += 1;
    }
    fs::write(out, text).map_err(|e| e.to_string())?;
    Ok(n)
}

/// The catalog's entries: (name, commit, toml).
fn read_catalog(text: &str) -> Vec<(String, String, String)> {
    let mut out: Vec<(String, String, String)> = Vec::new();
    for line in text.lines() {
        if let Some(head) = line.strip_prefix(CATALOG_MARK) {
            let mut w = head.split_whitespace();
            if let (Some(n), Some(c)) = (w.next(), w.next()) {
                out.push((n.to_owned(), c.to_owned(), String::new()));
                continue;
            }
        }
        if let Some(last) = out.last_mut() {
            last.2.push_str(line);
            last.2.push('\n');
        }
    }
    out
}

/// Startup: fetch the catalog (5 s at most) and update vendor/<name>'s
/// NEONBRIDGE.toml and UPSTREAM from it; new runtimes get a folder. Offline,
/// or a checkout (vendor/ beside a .git), or NEONBRIDGE_CATALOG=0: nothing changes.
pub fn sync_catalog(vendor: &Path) {
    if std::env::var("NEONBRIDGE_CATALOG").is_ok_and(|v| v.trim() == "0") || vendor.parent().is_some_and(|p| p.join(".git").exists()) {
        return;
    }
    let tmp = std::env::temp_dir().join("i95-catalog.txt");
    let ok = super::runtimes::command("curl").args(["-L", "--fail", "-sS", "--max-time", "5", "-o"]).arg(&tmp).arg(CATALOG_URL)
        .status().is_ok_and(|s| s.success());
    let Some(text) = ok.then(|| fs::read_to_string(&tmp).ok()).flatten() else {
        println!("[catalog] offline: using the runtimes shipped with this build");
        return;
    };
    let _ = fs::remove_file(&tmp);
    let mut changed = 0;
    for (name, commit, toml) in read_catalog(&text) {
        let safe = !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        if !safe || commit.len() != 40 || !commit.chars().all(|c| c.is_ascii_hexdigit()) || super::runtimes::check_spec(&toml).is_err() {
            println!("[catalog] {name}: skipped (bad entry)");
            continue;
        }
        let dir = vendor.join(&name);
        if fs::read_to_string(dir.join("NEONBRIDGE.toml")).is_ok_and(|t| t.replace("\r\n", "\n") == toml) && pin(&dir).as_deref() == Some(commit.as_str()) {
            continue;
        }
        // A changed build, prebuild, setup, diag_build or launcher line runs
        // commands on this PC: asked first, never taken silently.
        if let Ok(old) = fs::read_to_string(dir.join("NEONBRIDGE.toml"))
            && super::runtimes::commands_of(&old) != super::runtimes::commands_of(&toml)
            && !ask(&format!("The online runtime list changes the commands {name} builds, sets up or starts with.

Take the new commands? (No keeps the ones this PC has.)"))
        {
            println!("[catalog] {name}: changed commands declined; kept as it was");
            continue;
        }
        if fs::create_dir_all(&dir).is_ok() && fs::write(dir.join("NEONBRIDGE.toml"), &toml).is_ok() && fs::write(dir.join("UPSTREAM"), format!("{commit}\n")).is_ok() {
            changed += 1;
        }
    }
    println!("[catalog] {changed} runtime(s) updated from GitHub");
}

/// The exe-keyed shader cache line some forks have (klbf): every rebuild
/// threw its whole shader cache away. Replaced so the key is
/// NEONBRIDGE_SHADER_KEY (a hash of the translator's sources, set by the
/// launcher) when it is set, the exe as before when not.
const EXE_KEY: &str = "let exe = std::env::current_exe().and_then(std::fs::read);";
const SOURCE_KEY: &str = "let exe = match std::env::var(\"NEONBRIDGE_SHADER_KEY\") { Ok(k) => Ok(k.into_bytes()), Err(_) => std::env::current_exe().and_then(std::fs::read) }; // i95: keyed on the translator's sources, not the exe";

/// Where a tree keeps its shader cache key and translator.
const KEY_FILES: [(&str, &str); 3] = [
    ("crates/victory-gpu/src/shader_cache.rs", "crates/victory-shader"),
    ("third_party/victory/crates/victory-gpu/src/shader_cache.rs", "third_party/victory/crates/victory-shader"),
    ("runtime/victory/crates/victory-gpu/src/shader_cache.rs", "runtime/victory/crates/victory-shader"),
];

/// Makes `root`'s shader cache survive rebuilds when it keys on the whole exe:
/// patches the key line (once) and returns the translator folder to hash.
/// NEONBRIDGE_SHADER_KEY_PATCH=0 leaves trees as their authors ship them.
pub fn shader_key_patch(name: &str, root: &Path) -> Option<PathBuf> {
    // Off by default (runtimes run as their authors ship them); NEONBRIDGE_SHADER_KEY_PATCH=1 turns it on.
    if !std::env::var("NEONBRIDGE_SHADER_KEY_PATCH").is_ok_and(|v| v.trim() == "1") {
        return None;
    }
    for (file, shader) in KEY_FILES {
        let path = root.join(file);
        let Ok(text) = fs::read_to_string(&path) else { continue };
        if text.contains(EXE_KEY) {
            if fs::write(&path, text.replacen(EXE_KEY, SOURCE_KEY, 1)).is_ok() {
                println!("[fetch] {name}: shader cache keyed on its translator, not its exe (NEONBRIDGE_SHADER_KEY_PATCH=0 undoes it on a fresh copy)");
            }
            return Some(root.join(shader));
        }
        if text.contains(SOURCE_KEY) {
            return Some(root.join(shader));
        }
    }
    None
}

/// One hash of every file under `dir` (paths and bytes, sorted): changes only
/// when the translator's sources change.
pub fn tree_hash(dir: &Path) -> String {
    use sha2::{Digest, Sha256};
    fn walk(d: &Path, out: &mut Vec<PathBuf>) {
        if let Ok(rd) = fs::read_dir(d) {
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    if p.file_name().is_some_and(|n| n != "target") {
                        walk(&p, out);
                    }
                } else {
                    out.push(p);
                }
            }
        }
    }
    let mut files = Vec::new();
    walk(dir, &mut files);
    files.sort();
    let mut h = Sha256::new();
    for f in files {
        h.update(f.strip_prefix(dir).unwrap_or(&f).to_string_lossy().replace('\\', "/").as_bytes());
        h.update(fs::read(&f).unwrap_or_default());
    }
    format!("{:x}", h.finalize())
}

const RELEASE_API: &str = "https://api.github.com/repos/DesignWithTommy/i95-GTA6-PC-Launcher/releases/latest";

/// "v0.4.10" -> (0, 4, 10).
fn version(t: &str) -> Option<(u32, u32, u32)> {
    let mut p = t.trim().trim_start_matches('v').split('.').map(|n| n.parse::<u32>().ok());
    Some((p.next()??, p.next()??, p.next().flatten().unwrap_or(0)))
}

/// The first `"key": "value"` in GitHub's JSON (no escapes in tags or asset URLs).
fn json_str<'a>(json: &'a str, key: &str, from: usize) -> Option<(&'a str, usize)> {
    let at = from + json[from..].find(&format!("\"{key}\""))?;
    let rest = &json[at + key.len() + 2..];
    let open = rest.find('"')? + 1;
    let len = rest[open..].find('"')?;
    Some((&rest[open..open + len], at + key.len() + 2 + open + len))
}

/// Startup: the newest GitHub release (5 s at most). When it is newer than
/// this build, asked first: its zip is downloaded, i95.exe moved aside
/// (i95.exe.old, removed at the next start) and the zip's files copied over
/// this install, then the new i95 is started and this one exits.
/// A checkout build (a .git above the exe) or NEONBRIDGE_UPDATE=0: nothing.
pub fn self_update() {
    let Ok(exe) = std::env::current_exe() else { return };
    let Some(dir) = exe.parent().map(Path::to_path_buf) else { return };
    let _ = fs::remove_file(exe.with_extension("exe.old"));
    if std::env::var("NEONBRIDGE_UPDATE").is_ok_and(|v| v.trim() == "0") || dir.ancestors().any(|a| a.join(".git").exists()) {
        return;
    }
    let tmp = std::env::temp_dir().join("i95-release.json");
    let got = super::runtimes::command("curl").args(["-L", "--fail", "-sS", "--max-time", "5", "-H", "Accept: application/vnd.github+json", "-o"]).arg(&tmp).arg(RELEASE_API)
        .status().is_ok_and(|s| s.success());
    let Some(json) = got.then(|| fs::read_to_string(&tmp).ok()).flatten() else { return };
    let _ = fs::remove_file(&tmp);
    let Some((tag, _)) = json_str(&json, "tag_name", 0) else { return };
    let (Some(new), Some(ours)) = (version(tag), version(env!("CARGO_PKG_VERSION"))) else { return };
    if new <= ours {
        return;
    }
    let mut at = 0;
    let url = loop {
        match json_str(&json, "browser_download_url", at) {
            Some((u, _)) if u.ends_with(".zip") => break u.to_string(),
            Some((_, next)) => at = next,
            None => return,
        }
    };
    if !ask(&format!("i95 {tag} is out (this is v{}).\n\nUpdate now? i95 restarts when it is done.", env!("CARGO_PKG_VERSION"))) {
        return;
    }
    if let Err(e) = install_release(&url, &exe, &dir) {
        println!("[update] {tag}: {e}");
        if !exe.exists() {
            let _ = fs::rename(exe.with_extension("exe.old"), &exe);
        }
        ask(&format!("The update to {tag} failed: {e}\n\nThis version starts instead. (Yes or No both continue.)"));
        return;
    }
    println!("[update] {tag} installed; restarting");
    let _ = super::runtimes::command(&exe).current_dir(&dir).spawn();
    std::process::exit(0);
}

fn install_release(url: &str, exe: &Path, dir: &Path) -> Result<(), String> {
    let work = std::env::temp_dir().join("i95-update");
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let zip = work.join("i95.zip");
    crate::splash::status(0.05, "Downloading the i95 update");
    let ok = |c: &mut std::process::Command| c.status().is_ok_and(|s| s.success());
    if !ok(super::runtimes::command("curl").args(["-L", "--fail", "-sS", "-o"]).arg(&zip).arg(url)) {
        return Err(format!("download failed ({url})"));
    }
    let tar = PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into())).join(r"System32\tar.exe");
    if !ok(super::runtimes::command(&tar).arg("-xf").arg(&zip).arg("-C").arg(&work)) {
        return Err("unpacking failed".into());
    }
    // The zip holds one folder (i95\) with i95.exe in it.
    let top = fs::read_dir(&work).map_err(|e| e.to_string())?.flatten().map(|e| e.path()).find(|p| p.join("i95.exe").is_file())
        .ok_or("the zip held no i95.exe")?;
    // A running exe can be renamed, not overwritten.
    fs::rename(exe, exe.with_extension("exe.old")).map_err(|e| format!("moving the running exe aside: {e}"))?;
    copy_tree(&top, dir).map_err(|e| format!("copying the new files: {e}"))?;
    if top.join("i95.exe") != *exe && exe.file_name() != Some(std::ffi::OsStr::new("i95.exe")) {
        fs::copy(dir.join("i95.exe"), exe).map_err(|e| e.to_string())?;
    }
    let _ = fs::remove_dir_all(&work);
    Ok(())
}
