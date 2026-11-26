//! Diagnostics: what a dispatch of a named program read and wrote, as the
//! card held it right after the dispatch (`VICTORY_DISPATCH_DUMP=<folder>`).
//!
//! `VICTORY_DISPATCH_DUMP_NAMES` lists, comma-separated, texts to find in a
//! compute program's container (a program name such as
//! `CS_ScreenProbes_TemporalResolve`, or a hash). Once a file `go` is in the
//! folder, the next dispatch of each listed program has every view it reads
//! or writes through a descriptor table copied out after it ran, in the
//! dispatch's own list: a buffer view's bytes (at most [`MOST`]) from its
//! host copy, a texture's first subresource. A few frames later they are
//! written as `<n>-<text>-<srv|uav><param>_<dword>.bin`, with a line each
//! in `views.txt`: the file, the program, the resource's description and
//! the view's (the title's words), the row pitch of a texture's copy.
//! Each listed text is dumped once a run.
//!
//! The same views are copied before the dispatch too (`pre-` files), with a
//! `dispatch` line in `views.txt`: the group counts and the root arguments
//! (tables' addresses, root constants, root views); every view line carries
//! the stride word the host gives the view (`stride=`, as in
//! `SLOT_TABLE_STRIDES`), constant buffers and views of memory included, and
//! a view that cannot be copied gets a `skipped` line saying why.

use std::sync::{Mutex, OnceLock};

use victory_shader::{ResourceType, RootType};
use windows::Win32::Graphics::Direct3D12::*;

use crate::descriptor::View;
use crate::host::HostList;
use crate::object::This;

/// The most bytes copied of one buffer view.
const MOST: u64 = 64 << 20;
/// The most bytes copied for one dispatch.
const MOST_ALL: u64 = 768 << 20;
/// Indirect dispatches an `args:` text records at most.
const ARGS_MOST: u32 = 4000;
/// Frames after the dispatch when the copies are read.
const LATER: u64 = 8;

fn folder() -> Option<&'static std::path::PathBuf> {
    static FOLDER: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();
    FOLDER
        .get_or_init(|| {
            let folder: std::path::PathBuf = std::env::var_os("VICTORY_DISPATCH_DUMP")?.into();
            // What is dumped is the title's data: never inside a source tree (docs/asset-policy.md).
            if let Err(e) = victory_core::ensure_outside_repo(&folder) {
                tracing::error!(%e, "VICTORY_DISPATCH_DUMP refused: nothing is dumped");
                return None;
            }
            Some(folder)
        })
        .as_ref()
}

fn names() -> &'static [String] {
    static NAMES: OnceLock<Vec<String>> = OnceLock::new();
    NAMES.get_or_init(|| {
        if folder().is_none() {
            return Vec::new();
        }
        std::env::var("VICTORY_DISPATCH_DUMP_NAMES")
            .unwrap_or_default()
            .split(',')
            .map(|n| n.trim().to_owned())
            .filter(|n| !n.is_empty())
            .collect()
    })
}

/// The listed text a compute program's container holds, if any.
pub(crate) fn role(container: &[u8], hash: &str) -> Option<u8> {
    names()
        .iter()
        .position(|n| {
            let n = n.strip_prefix("args:").unwrap_or(n);
            hash.starts_with(n) || container.windows(n.len()).any(|w| w == n.as_bytes())
        })
        .map(|n| n as u8)
}

/// One view copied out.
struct Copied {
    /// Empty: the copy's dwords go into its line in `views.txt` instead.
    file: String,
    line: String,
    buffer: ID3D12Resource,
    data: usize,
    len: usize,
}

#[derive(Default)]
struct State {
    done: Vec<u8>,
    /// Placed textures whose alias copy was copied out.
    aliases: Vec<usize>,
    /// Indirect dispatches recorded so far by `args:` text.
    args: std::collections::HashMap<u8, u32>,
    due: Vec<(u64, Vec<Copied>)>,
    count: usize,
    /// Dispatches dumped so far by listed text (`VICTORY_DISPATCH_DUMP_REPEAT`).
    repeats: std::collections::HashMap<u8, u32>,
}

// SAFETY: the mappings are of buffers the state holds.
unsafe impl Send for State {}

static STATE: Mutex<Option<State>> = Mutex::new(None);

/// Whether listed text `k` is to be dumped at this dispatch.
pub(crate) fn armed(k: u8) -> bool {
    let Some(folder) = folder() else { return false };
    let state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    if state.as_ref().is_some_and(|s| s.done.contains(&k)) {
        return false;
    }
    drop(state);
    let go = folder.join("go");
    if go.exists() {
        return true;
    }
    if f9_pressed() {
        let _ = std::fs::write(&go, b"");
        tracing::warn!("dispatch dump: F9 pressed, dumping from now on");
        return true;
    }
    false
}

/// (local) F9 with one of our windows in front arms the dump like creating `go` does. Polled at
/// most every 50 ms: under Wine a key query is a server call and this runs at every dispatch.
fn f9_pressed() -> bool {
    use std::sync::atomic::{AtomicU64, Ordering};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_F9};
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
    static LAST_MS: AtomicU64 = AtomicU64::new(0);
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    let now = START.get_or_init(std::time::Instant::now).elapsed().as_millis() as u64;
    let last = LAST_MS.load(Ordering::Relaxed);
    if now < last + 50 || LAST_MS.compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed).is_err() {
        return false;
    }
    // SAFETY: plain queries; room for the answer.
    let down = unsafe { GetAsyncKeyState(i32::from(VK_F9)) } < 0;
    if !down {
        return false;
    }
    let mut process = 0;
    unsafe { GetWindowThreadProcessId(GetForegroundWindow(), &mut process) };
    process == std::process::id()
}

/// A view a dispatch read or wrote through a table.
pub(crate) struct Seen {
    pub(crate) kind: ResourceType,
    pub(crate) slot: u8,
    pub(crate) dword: u32,
    pub(crate) view: View,
}

/// Records the copies of `views`, the views the dispatch of listed text `k`
/// that the list just recorded reads and writes.
pub(crate) fn record(
    list: &HostList,
    k: u8,
    hash: &str,
    views: Vec<Seen>,
    arguments: Option<(&ID3D12Resource, u64)>,
    before: bool,
    info: &str,
) {
    let Some(host) = crate::host::host() else { return };
    let name = names()[usize::from(k)].clone();
    // `args:<text>`: the group counts of every indirect dispatch of the
    // program, for the first ARGS_MOST of them, into `views.txt`.
    if name.starts_with("args:") {
        if before {
            return;
        }
        let Some((from, at)) = arguments else { return };
        let Some((to, data)) = host.shared_buffer(12) else { return };
        // SAFETY: an open list; live buffers; the record lies in the scratch buffer.
        unsafe { list.list.CopyBufferRegion(&to, 0, from, at, 12) };
        list.keep(&to);
        let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let state = guard.get_or_insert_with(Default::default);
        let seen = state.args.entry(k).or_insert(0);
        *seen += 1;
        if *seen >= ARGS_MOST {
            state.done.push(k);
        }
        let frame = crate::frame::current();
        state.due.push((
            frame + LATER,
            vec![Copied {
                file: String::new(),
                line: format!("args {name} {hash} frame {frame} groups"),
                buffer: to,
                data: data as usize,
                len: 12,
            }],
        ));
        return;
    }
    let mut copied = Vec::new();
    // Every earlier shader write done.
    let uav = D3D12_RESOURCE_BARRIER { Type: D3D12_RESOURCE_BARRIER_TYPE_UAV, ..Default::default() };
    // SAFETY: an open list; the barrier names no resource.
    unsafe { list.list.ResourceBarrier(&[uav]) };
    // File numbers for the views, taken at once (an alias copy may take
    // some meanwhile).
    let n0 = {
        let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let state = guard.get_or_insert_with(Default::default);
        let n = state.count;
        state.count += views.len() + 2;
        n
    };
    let mut n = n0;
    let when = if before { "pre" } else { "post" };
    let mut notes = vec![format!("{n:03} dispatch {name} {hash} {when} {info}")];
    n += 1;
    // An indirect dispatch's three group counts, as the card read them.
    if let Some((from, at)) = arguments
        && let Some((to, data)) = host.shared_buffer(12)
    {
        let file = format!("{n:03}-{when}-{}-arguments.bin", name.replace(['/', '\\', ':'], "_"));
        // SAFETY: an open list; live buffers; the record lies in the scratch buffer.
        unsafe { list.list.CopyBufferRegion(&to, 0, from, at, 12) };
        list.keep(&to);
        copied.push(Copied {
            line: format!("{file} {name} {hash} arguments indirect dispatch group counts (3 dwords)"),
            file,
            buffer: to,
            data: data as usize,
            len: 12,
        });
        n += 1;
    }
    let mut all = 0u64;
    for seen in views {
        if all >= MOST_ALL {
            tracing::warn!(name, "dispatch dump: more than is copied for one dispatch; the rest left out");
            break;
        }
        let what = match seen.kind {
            ResourceType::Uav => "uav",
            ResourceType::Cbv => "cbv",
            _ => "srv",
        };
        let file = format!(
            "{n:03}-{when}-{}-{what}{}_{}.bin",
            name.replace(['/', '\\', ':'], "_"),
            seen.slot,
            seen.dword
        );
        let stride = crate::descriptor::host::view_stride_word(&seen.view);
        let range = match &seen.view {
            View::Constant(address, size) => Some((*address, u64::from(*size))),
            View::Placed(..) => crate::descriptor::host::buffer_range(&seen.view),
            View::Shader(r, _) | View::Unordered(r, _, _) if crate::resource::buffer_bytes(*r as This).is_some() => {
                crate::descriptor::host::buffer_range(&seen.view)
            }
            View::Shader(..) | View::Unordered(..) => None,
            _ => {
                notes.push(format!("skipped {what}{}_{} not a buffer or texture view", seen.slot, seen.dword));
                continue;
            }
        };
        let view_words = match &seen.view {
            View::Shader(_, w) | View::Unordered(_, _, w) => w.map_or_else(|| "own".to_owned(), |w| format!("{w:x?}")),
            View::Placed(u, r, w) => format!("placed uav={u} resource={r:x?} view={w:x?}"),
            View::Constant(a, l) => format!("constant {a:#x} {l}"),
            _ => String::new(),
        };
        if let Some((address, len)) = range {
            let Some(shadow) = crate::shadow::find(address, len.max(1)) else {
                notes.push(format!(
                    "skipped {what}{}_{} no host copy of {address:#x}+{len} stride={stride:#x} view={view_words}",
                    seen.slot, seen.dword
                ));
                continue;
            };
            // SAFETY: a live buffer.
            let base = unsafe { shadow.buffer.GetGPUVirtualAddress() };
            let offset = shadow.gpu(address) - base;
            // SAFETY: a live buffer.
            let size = unsafe { shadow.buffer.GetDesc() }.Width;
            let len = len.min(MOST).min(size.saturating_sub(offset));
            if len == 0 {
                notes.push(format!("skipped {what}{}_{} empty at {address:#x}", seen.slot, seen.dword));
                continue;
            }
            let Some((to, data)) = host.shared_buffer(len) else { continue };
            // SAFETY: an open list; live buffers; the range lies in both.
            unsafe { list.list.CopyBufferRegion(&to, 0, &shadow.buffer, offset, len) };
            list.keep(&to);
            copied.push(Copied {
                line: format!(
                    "{file} {name} {hash} buffer address={address:#x} bytes={len} stride={stride:#x} view={view_words}"
                ),
                file,
                buffer: to,
                data: data as usize,
                len: len as usize,
            });
            all += len;
        } else {
            let resource = match &seen.view {
                View::Shader(r, _) | View::Unordered(r, _, _) => *r as This,
                _ => {
                    notes.push(format!("skipped {what}{}_{} no range", seen.slot, seen.dword));
                    continue;
                }
            };
            let Some(texture) = crate::resource::host_resource(resource) else {
                notes.push(format!("skipped {what}{}_{} no host texture", seen.slot, seen.dword));
                continue;
            };
            let texture = texture.clone();
            let Some(c) = texture_copy(list, host, resource, &texture, file, &format!("{name} {hash}"), &view_words) else {
                continue;
            };
            let bytes = c.len as u64;
            copied.push(c);
            all += bytes;
        }
        n += 1;
    }
    // The notes go out as lines of their own (a copy of no bytes).
    let notes: Vec<Copied> = notes
        .into_iter()
        .filter_map(|line| {
            let (to, data) = host.shared_buffer(4)?;
            Some(Copied { file: String::new(), line, buffer: to, data: data as usize, len: 0 })
        })
        .collect();
    copied.splice(0..0, notes);
    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let state = state.get_or_insert_with(Default::default);
    if !before {
        // `VICTORY_DISPATCH_DUMP_REPEAT=<n>`: the first n dispatches of each name (default 1).
        let repeat = std::env::var("VICTORY_DISPATCH_DUMP_REPEAT").ok().and_then(|v| v.parse().ok()).unwrap_or(1u32);
        let seen = state.repeats.entry(k).or_insert(0);
        *seen += 1;
        if *seen >= repeat {
            state.done.push(k);
        }
    }
    tracing::info!(name, before, views = copied.len(), "dispatch dump: views copied");
    state.due.push((crate::frame::current() + LATER, copied));
}

/// A copy of `texture`'s first subresource (the host copy of title
/// texture `resource`), recorded into the list.
fn texture_copy(
    list: &HostList,
    host: &crate::host::Host,
    resource: This,
    texture: &ID3D12Resource,
    file: String,
    what: &str,
    view_words: &str,
) -> Option<Copied> {
    // SAFETY: a live texture.
    let d = unsafe { texture.GetDesc() };
    if d.SampleDesc.Count > 1 {
        return None;
    }
    let mut footprint = D3D12_PLACED_SUBRESOURCE_FOOTPRINT::default();
    let mut bytes = 0u64;
    // SAFETY: a plain query into locals.
    unsafe {
        host.device.GetCopyableFootprints(&d, 0, 1, 0, Some(&mut footprint), None, None, Some(&mut bytes));
    }
    if bytes == 0 || bytes > 4 * MOST {
        return None;
    }
    let (to, data) = host.shared_buffer(bytes)?;
    crate::list::ensure_state(list, resource, 0x800);
    let source = D3D12_TEXTURE_COPY_LOCATION {
        // SAFETY: a borrowed pointer the location never releases.
        pResource: unsafe { std::mem::transmute_copy(texture) },
        Type: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
        Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 { SubresourceIndex: 0 },
    };
    let dest = D3D12_TEXTURE_COPY_LOCATION {
        // SAFETY: as above.
        pResource: unsafe { std::mem::transmute_copy(&to) },
        Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
        Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 { PlacedFootprint: footprint },
    };
    // SAFETY: an open list; the texture is a copy source now.
    unsafe { list.list.CopyTextureRegion(&dest, 0, 0, 0, &source, None) };
    list.keep(&to);
    list.keep(texture);
    let f = footprint.Footprint;
    Some(Copied {
        line: format!(
            "{file} {what} texture {}x{}x{} levels={} format={} dimension={} copy_format={} row_pitch={} view={view_words}",
            d.Width, d.Height, d.DepthOrArraySize, d.MipLevels, d.Format.0, d.Dimension.0, f.Format.0, f.RowPitch
        ),
        file,
        buffer: to,
        data: data as usize,
        len: bytes as usize,
    })
}

/// With `alias` among the names: right after `crate::texture::alias`
/// copied `source`'s texels into `placed` (both title textures; `from`
/// and `to` their host copies), both are copied out, for the first 24
/// such copies after `go` (each placed texture once).
pub(crate) fn alias(list: &HostList, placed: This, source: This, from: &ID3D12Resource, to: &ID3D12Resource) {
    let Some(k) = names().iter().position(|n| n == "alias") else { return };
    let k = k as u8;
    if !armed(k) {
        return;
    }
    let Some(host) = crate::host::host() else { return };
    let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let state = guard.get_or_insert_with(Default::default);
    let key = placed as usize;
    if state.aliases.contains(&key) {
        return;
    }
    state.aliases.push(key);
    if state.aliases.len() >= 24 {
        state.done.push(k);
    }
    let n = state.count;
    state.count += 2;
    drop(guard);
    let mut copied = Vec::new();
    let what = format!("alias placed={placed:?} source={source:?}");
    for (i, (this, texture)) in [(source, from), (placed, to)].into_iter().enumerate() {
        let file = format!("{:03}-alias-{}.bin", n + i, if i == 0 { "source" } else { "placed" });
        if let Some(c) = texture_copy(list, host, this, texture, file, &what, "-") {
            copied.push(c);
        }
    }
    tracing::info!(?placed, ?source, "dispatch dump: an alias copy copied out");
    let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
    state.get_or_insert_with(Default::default).due.push((crate::frame::current() + LATER, copied));
}

/// Writes out the copies whose frames have run.
pub(crate) fn poll() {
    let Some(folder) = folder() else { return };
    let ready = {
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());
        let Some(state) = state.as_mut() else { return };
        let now = crate::frame::current();
        let (ready, wait): (Vec<_>, Vec<_>) = std::mem::take(&mut state.due).into_iter().partition(|d| d.0 <= now);
        state.due = wait;
        ready
    };
    for (_, copies) in ready {
        let mut lines = String::new();
        for c in &copies {
            // SAFETY: a mapping of `len` bytes; the copy ran frames ago.
            let bytes = unsafe { std::slice::from_raw_parts(c.data as *const u8, c.len) };
            lines += &c.line;
            if c.file.is_empty() {
                for w in bytes.chunks_exact(4) {
                    lines += &format!(" {}", u32::from_le_bytes([w[0], w[1], w[2], w[3]]));
                }
            } else {
                let _ = std::fs::write(folder.join(&c.file), bytes);
            }
            lines.push('\n');
            let _ = &c.buffer;
        }
        use std::io::Write as _;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(folder.join("views.txt")) {
            let _ = f.write_all(lines.as_bytes());
        }
        if copies.iter().any(|c| !c.file.is_empty()) {
            tracing::info!(files = copies.len(), "dispatch dump: written");
        }
    }
}

/// The views a translated program reads and writes through tables, as the
/// bound tables hold them.
pub(crate) fn seen(t: &crate::pipeline::Translated, table: impl Fn(u8) -> Option<u64>) -> Vec<Seen> {
    let mut out = Vec::new();
    for u in &t.used {
        if u.root != RootType::Table || !matches!(u.kind, ResourceType::Srv | ResourceType::Uav | ResourceType::Cbv) {
            continue;
        }
        let Some(start) = table(u.slot) else { continue };
        if out.iter().any(|s: &Seen| (s.slot, s.dword) == (u.slot, u.table_offset_dwords)) {
            continue;
        }
        let Some(view) = crate::descriptor::view(start + u64::from(u.table_offset_dwords) * 4) else { continue };
        out.push(Seen { kind: u.kind, slot: u.slot, dword: u.table_offset_dwords, view });
    }
    out
}
