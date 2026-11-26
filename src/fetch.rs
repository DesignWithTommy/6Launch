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
    ["Cargo.toml", "apps", "crates", "third_party", "src", "runtime"].iter().any(|p| dir.join(p).exists())
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
