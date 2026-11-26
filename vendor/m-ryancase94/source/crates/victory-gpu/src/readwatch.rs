//! `VICTORY_READ_WATCH=<level>` (diagnostics builds): which of the title's own
//! code reads title memory of textures the GPU writes. The host keeps what
//! shaders write to a texture in its host copy and copies back only the kinds
//! `crate::texback` is told to, so title code that reads any other such
//! texture on the CPU reads bytes nothing wrote. This looks for them; it
//! copies nothing back.
//!
//! What is watched, by level:
//! - `1`: textures written through unordered access (resources and views of
//!   placed memory);
//! - `2`: also colour render targets;
//! - `3`: also depth targets and the destinations of copies and resolves.
//! - `VICTORY_READ_WATCH_BUFFERS=1` (with any level): also the ranges of
//!   buffers written through unordered access views.
//!
//! How: the pages of each watched range of title memory get `PAGE_GUARD`,
//! which the system clears at the first touch of a page, raising an exception
//! our handler (first in line) takes: it notes the range, the code address,
//! the address touched and whether it was a read or a write, and lets the
//! access go on. A thread of ours takes the notes every `VICTORY_READ_WATCH_MS`
//! (default 1000), counts them per kind of texture, and puts the guards
//! back. So a count here is a count of GUARD HITS: at most one per 4 KiB page
//! and interval, however often the title reads the page in between.
//!
//! What it reports:
//! - a warning line the first time a code address of the title reads (or
//!   writes) a kind of texture: code as `<image file>+<offset>`, the kind as
//!   `<width>x<height>x<depth or array> mips <n> f<DXGI format>`, the texture's
//!   address in title memory and the offset touched;
//! - every `VICTORY_READ_WATCH_SUMMARY_S` seconds (default 10) and at exit, a
//!   table with one line per kind: distinct textures watched, distinct
//!   textures read, read hits, write hits, distinct code addresses that read,
//!   hits through another view of the pages, hits from code that is not the
//!   title's, and whether `VICTORY_TEXTURE_BACK` covers the kind (`default`:
//!   in its default list; `set`: in the list this run uses; `no`). The kinds
//!   the title read go to the log; the whole table with every code address
//!   goes to `VICTORY_READ_WATCH_FILE=<file>`, written anew each time.
//!
//! A range is let go (and its guards taken off) when its resource is
//! released; a view of placed memory, when a new range is put over it. The host's own reads
//! and writes of title memory through the kernel take the guards off first
//! (a system call that meets a guarded page fails) and put them back after.
//!
//! shortcuts: only what is touched while a guard stands is seen: after a
//! write hit, reads of the same page go unseen until the next interval. Code
//! of the title's that reads through a system library (a `memcpy` of the C
//! runtime's) is counted as other code, named by its module. A view of placed
//! memory has no resource whose release ends the watch: it lasts until its
//! memory is watched as something else. A buffer range shares its first and
//! last pages with its neighbours: touches there are not counted, but they
//! cost a hit.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock, RwLock};

use crate::object::This;

/// 0 off, else what is watched (see the module's text).
pub(crate) fn level() -> u8 {
    static L: OnceLock<u8> = OnceLock::new();
    *L.get_or_init(|| {
        std::env::var("VICTORY_READ_WATCH").ok().and_then(|v| v.trim().parse().ok()).unwrap_or(0)
    })
}

/// Whether buffer ranges written through unordered access are watched too.
fn buffers() -> bool {
    static B: OnceLock<bool> = OnceLock::new();
    *B.get_or_init(|| level() != 0 && std::env::var("VICTORY_READ_WATCH_BUFFERS").is_ok_and(|v| v.trim() == "1"))
}

/// Whether the other views of a range's pages are watched too
/// (`VICTORY_READ_WATCH_VIEWS=0`: not).
fn views() -> bool {
    static V: OnceLock<bool> = OnceLock::new();
    *V.get_or_init(|| !std::env::var("VICTORY_READ_WATCH_VIEWS").is_ok_and(|v| v.trim() == "0"))
}

fn number(name: &str, default: u64, least: u64) -> u64 {
    std::env::var(name).ok().and_then(|v| v.trim().parse().ok()).unwrap_or(default).max(least)
}

/// A kind of texture: width, height, depth or array size, mip levels, DXGI
/// format. Of a buffer range: bytes, stride, 0, 0, the view's format.
pub(crate) type Kind = [u64; 5];

fn kind_text(kind: &Kind, buffer: bool) -> String {
    if buffer {
        format!("buffer {} bytes stride {} f{}", kind[0], kind[1], kind[4])
    } else {
        format!("{}x{}x{} mips {} f{}", kind[0], kind[1], kind[2], kind[3], kind[4])
    }
}

const MOST: usize = 16384;
/// The watched ranges (start, end), written under `STATE`'s lock and read
/// without one by the exception handler. An end of 0: no range.
static STARTS: [AtomicUsize; MOST] = [const { AtomicUsize::new(0) }; MOST];
static ENDS: [AtomicUsize; MOST] = [const { AtomicUsize::new(0) }; MOST];
/// One more than the highest slot ever used.
static HIGH: AtomicUsize = AtomicUsize::new(0);

/// Hits not counted yet, written by the exception handler without a lock or
/// an allocation: slot | access << 32 | 1 << 63 (written last), code address,
/// address touched.
const RING: usize = 1 << 17;
static HIT_WHAT: [AtomicU64; RING] = [const { AtomicU64::new(0) }; RING];
static HIT_CODE: [AtomicUsize; RING] = [const { AtomicUsize::new(0) }; RING];
static HIT_AT: [AtomicUsize; RING] = [const { AtomicUsize::new(0) }; RING];
static HIT_HEAD: AtomicU64 = AtomicU64::new(0);
const NOTED: u64 = 1 << 63;

/// The 64 KiB blocks that hold watched pages, with how many ranges each
/// holds: what the host's kernel copies ask before they run.
static BLOCKS: RwLock<Option<HashMap<usize, u32>>> = RwLock::new(None);

struct Slot {
    /// The texture (or buffer range) it belongs to: its start and length.
    key: (usize, usize),
    /// For another view of the texture's pages: this view's start less the
    /// texture's address shown there.
    view_of: Option<usize>,
    /// The tick it was let go at; freed two ticks later (a hit on its way to
    /// the handler still finds it).
    dead: Option<u64>,
}

struct Watched {
    kind: Kind,
    buffer: bool,
    slots: Vec<usize>,
    /// The resources that are this memory; none for a view of placed memory.
    owners: HashSet<usize>,
}

#[derive(Default)]
struct Stat {
    how: std::collections::BTreeSet<&'static str>,
    textures: HashSet<usize>,
    read_textures: HashSet<usize>,
    reads: u64,
    writes: u64,
    via_view: u64,
    other: u64,
    /// Per code address of the title's: read hits, write hits.
    codes: BTreeMap<String, [u64; 2]>,
    /// Per code address that is not the title's: hits.
    others: BTreeMap<String, u64>,
}

#[derive(Default)]
struct State {
    slots: Vec<Option<Slot>>,
    free: Vec<usize>,
    watched: HashMap<(usize, usize), Watched>,
    by_owner: HashMap<usize, Vec<(usize, usize)>>,
    stats: BTreeMap<(Kind, bool), Stat>,
    tick: u64,
    /// Hits counted up to here.
    tail: u64,
    lost: u64,
    full: u64,
    beside: u64,
    changed: bool,
    /// Images: (start, end, file name, the title's).
    images: Vec<(usize, usize, String, bool)>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn state() -> std::sync::MutexGuard<'static, Option<State>> {
    let mut s = STATE.lock().unwrap_or_else(|e| e.into_inner());
    s.get_or_insert_with(State::default);
    s
}

/// What is known already, so that a draw asks nothing twice: resources (with
/// how they are written), buffer ranges, views of placed memory.
#[derive(Default)]
struct Known {
    resources: HashSet<(usize, &'static str)>,
    ranges: HashSet<(usize, u64, u64)>,
    placed: HashSet<(u64, [u32; 14])>,
}
static KNOWN: Mutex<Option<Known>> = Mutex::new(None);

fn known<R>(f: impl FnOnce(&mut Known) -> R) -> R {
    f(KNOWN.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(Known::default))
}

fn level_of(how: &str) -> u8 {
    match how {
        "unordered" | "buffer" => 1,
        "target" => 2,
        _ => 3,
    }
}

/// A draw, dispatch or copy about to be recorded writes title texture
/// `resource` (`how`: "unordered", "target", "depth", "copy").
pub(crate) fn gpu_writes(resource: This, how: &'static str) {
    let level = level();
    if level == 0 || level < level_of(how) || resource.is_null() {
        return;
    }
    if !known(|k| k.resources.insert((resource as usize, how))) {
        return;
    }
    let Some((at, len, kind)) = crate::resource::title_span(resource) else {
        return;
    };
    watch(at, len, kind, false, how, resource as usize);
}

/// A draw or dispatch about to be recorded binds an unordered access view of
/// `resource` with the view description `desc` (as words).
pub(crate) fn gpu_writes_view(resource: This, desc: Option<&[u32; 10]>) {
    if level() == 0 || resource.is_null() {
        return;
    }
    let Some((data, bytes)) = crate::resource::buffer_bytes(resource) else {
        gpu_writes(resource, "unordered");
        return;
    };
    if !buffers() {
        return;
    }
    let (from, len, stride, format) = buffer_range(desc, bytes);
    if !known(|k| k.ranges.insert((resource as usize, from, len))) {
        return;
    }
    watch(data as usize + from as usize, len as usize, [len, stride, 0, 0, format], true, "buffer", resource as usize);
}

/// The bytes of a buffer of `bytes` bytes an unordered access view with the
/// description `desc` shows: offset, length, stride, format. The whole
/// buffer if the description does not say.
fn buffer_range(desc: Option<&[u32; 10]>, bytes: u64) -> (u64, u64, u64, u64) {
    const DIMENSION_BUFFER: u32 = 1;
    const RAW: u32 = 1;
    let Some(d) = desc.filter(|d| d[1] == DIMENSION_BUFFER) else {
        return (0, bytes, 0, 0);
    };
    let (first, count, stride, flags) = (u64::from(d[2]) | u64::from(d[3]) << 32, u64::from(d[4]), u64::from(d[5]), d[8]);
    let element = match (stride, flags & RAW != 0, d[0]) {
        (s, _, _) if s != 0 => s,
        (_, true, _) => 4,
        // Typed, by the DXGI format's bytes: the R32G32B32A32 formats, the
        // R32G32B32 ones, R16G16B16A16 to R32G8X24, R10G10B10A2 to R24G8,
        // R8G8 and R16, R8.
        (_, _, 1..=4) => 16,
        (_, _, 5..=8) => 12,
        (_, _, 9..=22) => 8,
        (_, _, 23..=47) => 4,
        (_, _, 48..=59) => 2,
        (_, _, 60..=65) => 1,
        _ => return (0, bytes, 0, u64::from(d[0])),
    };
    let from = first.saturating_mul(element).min(bytes);
    let len = count.saturating_mul(element).min(bytes - from);
    (from, len, element, u64::from(d[0]))
}

/// A draw or dispatch about to be recorded binds an unordered access view of
/// memory the title placed itself (resource description and view description
/// as words; the view's words 10 and 11 are the memory's address).
pub(crate) fn gpu_writes_placed(resource: &[u32; 14], view: &[u32; 16]) {
    if level() == 0 {
        return;
    }
    let address = u64::from(view[10]) | u64::from(view[11]) << 32;
    if address == 0 || !known(|k| k.placed.insert((address, *resource))) {
        return;
    }
    const BUFFER: u32 = 1;
    if resource[0] == BUFFER {
        if !buffers() {
            return;
        }
        // The description's width (words 4 and 5) is the buffer's bytes.
        let bytes = u64::from(resource[4]) | u64::from(resource[5]) << 32;
        let desc: [u32; 10] = view[..10].try_into().unwrap_or_default();
        let (from, len, stride, format) = buffer_range(Some(&desc), bytes);
        watch(address as usize + from as usize, len as usize, [len, stride, 0, 0, format], true, "placed buffer", 0);
    } else if let Some((len, kind)) = crate::resource::placed_span(resource) {
        watch(address as usize, len, kind, false, "placed", 0);
    }
}

/// Title resource `resource` is released: what was watched for it alone is
/// let go.
pub(crate) fn dropped(resource: This) {
    if HIGH.load(Ordering::Relaxed) == 0 {
        return;
    }
    let owner = resource as usize;
    known(|k| {
        k.resources.retain(|&(r, _)| r != owner);
        k.ranges.retain(|&(r, ..)| r != owner);
    });
    let mut s = state();
    let s = s.as_mut().expect("made by state()");
    for key in s.by_owner.remove(&owner).unwrap_or_default() {
        let gone = s.watched.get_mut(&key).is_some_and(|w| {
            w.owners.remove(&owner);
            w.owners.is_empty()
        });
        if gone {
            let_go(s, key);
        }
    }
}

/// Ranges longer than this are not watched (counted as not watched).
const LONGEST: usize = 1 << 30;

/// Watches `[at, at + len)` as a texture (or buffer range) of `kind`.
fn watch(at: usize, len: usize, kind: Kind, buffer: bool, how: &'static str, owner: usize) {
    if at == 0 || len == 0 {
        return;
    }
    let mut guard_now = Vec::new();
    {
        let mut s = state();
        let s = s.as_mut().expect("made by state()");
        let stat = s.stats.entry((kind, buffer)).or_default();
        stat.how.insert(how);
        stat.textures.insert(at);
        s.changed = true;
        let key = (at, len);
        // The same memory watched already: as the same kind, or as another
        // kind of a live resource's (then it stays what it was first seen
        // as, and both kinds are in the table).
        if let Some(w) = s.watched.get_mut(&key)
            && ((w.kind, w.buffer) == (kind, buffer) || !w.owners.is_empty())
        {
            if owner != 0 && w.owners.insert(owner) {
                s.by_owner.entry(owner).or_default().push(key);
            }
            return;
        }
        if len > LONGEST {
            s.full += 1;
            return;
        }
        // A view of placed memory watched here is something else now (it has
        // no resource whose release would say so). Ranges of live resources
        // stay: two that share memory are both watched, and a hit there
        // counts for the one found first.
        let over: Vec<(usize, usize)> = s
            .watched
            .iter()
            .filter(|&(&(a, l), w)| w.owners.is_empty() && a < at + len && at < a + l)
            .map(|(&key, _)| key)
            .collect();
        for old in over {
            let_go(s, old);
        }
        install(s);
        // The range itself, then the other views of its pages.
        let mut ranges = vec![(at, at + len, None)];
        if views() {
            ranges.extend(other_views(at, len).into_iter().map(|(from, to, shown)| (from, to, Some(shown))));
        }
        let mut slots = Vec::new();
        for (from, to, shown) in ranges {
            let Some(i) = s.free.pop().or_else(|| {
                let n = s.slots.len();
                (n < MOST).then(|| {
                    s.slots.push(None);
                    n
                })
            }) else {
                s.full += 1;
                continue;
            };
            s.slots[i] = Some(Slot { key, view_of: shown.map(|shown: usize| from.wrapping_sub(shown)), dead: None });
            STARTS[i].store(from, Ordering::Relaxed);
            ENDS[i].store(to, Ordering::Release);
            HIGH.fetch_max(i + 1, Ordering::AcqRel);
            slots.push(i);
            guard_now.push((from, to));
        }
        if slots.is_empty() {
            return;
        }
        {
            let mut blocks = BLOCKS.write().unwrap_or_else(|e| e.into_inner());
            let blocks = blocks.get_or_insert_with(HashMap::new);
            for &(from, to) in &guard_now {
                for b in from >> 16..=(to - 1) >> 16 {
                    *blocks.entry(b).or_insert(0) += 1;
                }
            }
        }
        let mut owners = HashSet::new();
        if owner != 0 {
            owners.insert(owner);
            s.by_owner.entry(owner).or_default().push(key);
        }
        s.watched.insert(key, Watched { kind, buffer, slots, owners });
    }
    for (from, to) in guard_now {
        set_guard(from, to, true);
    }
}

/// The other views of `[at, at + len)`'s pages: (start, end, the address in
/// the range shown at start), runs joined. None if a page is shown in many.
fn other_views(at: usize, len: usize) -> Vec<(usize, usize, usize)> {
    const K64: usize = 64 << 10;
    let end = at + len;
    let mut out: Vec<(usize, usize, usize)> = Vec::new();
    let mut page = at & !(K64 - 1);
    while page < end {
        let shown = crate::inplace::pages_of(page);
        if shown.len() > 8 {
            return Vec::new();
        }
        let (from, to) = (at.max(page), end.min(page + K64));
        for other in shown.into_iter().filter(|&o| o != page) {
            let (start, stop) = (other + (from - page), other + (to - page));
            match out.iter_mut().find(|r| r.1 == start && r.2 + (r.1 - r.0) == from) {
                Some(run) => run.1 = stop,
                None => out.push((start, stop, from)),
            }
        }
        page += K64;
    }
    out
}

/// Lets the range `key` go: its guards off now, its slots free two ticks on.
fn let_go(s: &mut State, key: (usize, usize)) {
    let Some(w) = s.watched.remove(&key) else {
        return;
    };
    let mut blocks = BLOCKS.write().unwrap_or_else(|e| e.into_inner());
    for i in w.slots {
        let (from, to) = (STARTS[i].load(Ordering::Relaxed), ENDS[i].load(Ordering::Relaxed));
        if let Some(slot) = s.slots[i].as_mut() {
            slot.dead = Some(s.tick);
        }
        if to == 0 {
            continue;
        }
        if let Some(blocks) = blocks.as_mut() {
            for b in from >> 16..=(to - 1) >> 16 {
                if let Some(n) = blocks.get_mut(&b) {
                    *n -= 1;
                    if *n == 0 {
                        blocks.remove(&b);
                    }
                }
            }
        }
        set_guard(from, to, false);
    }
}

/// Puts `PAGE_GUARD` on the committed, accessible pages of `[from, to)`, or
/// takes it off them; whether it changed any page.
fn set_guard(from: usize, to: usize, on: bool) -> bool {
    use windows_sys::Win32::System::Memory::{
        MEM_COMMIT, MEMORY_BASIC_INFORMATION, PAGE_GUARD, PAGE_NOACCESS, VirtualProtect, VirtualQuery,
    };
    let end = (to + 0xfff) & !0xfff;
    let mut p = from & !0xfff;
    let mut changed = false;
    while p < end {
        // SAFETY: plain data filled in by the query.
        let mut info: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: a valid out pointer of the right size.
        if unsafe { VirtualQuery(p as *const _, &mut info, size_of_val(&info)) } == 0 {
            break;
        }
        let region_end = (info.BaseAddress as usize).saturating_add(info.RegionSize).min(end);
        // (no guard with no access, uncached or write-combined pages)
        const NOT_WITH_GUARD: u32 = PAGE_NOACCESS | 0x200 | 0x400;
        let guarded = info.Protect & PAGE_GUARD != 0;
        if info.State == MEM_COMMIT && info.Protect & NOT_WITH_GUARD == 0 && info.Protect != 0 && guarded != on {
            let protect = if on { info.Protect | PAGE_GUARD } else { info.Protect & !PAGE_GUARD };
            let mut old = 0;
            // SAFETY: committed pages of the title's; the protection keeps
            // their access and adds or drops the one-shot guard.
            changed |= unsafe { VirtualProtect(p as *const _, region_end - p, protect, &mut old) } != 0;
        }
        p = region_end.max(p + 0x1000);
    }
    changed
}

/// Before the kernel reads or writes title memory `[at, at + len)` for the
/// host or the runtime (a system call that meets a guarded page fails, and a
/// copy of ours would take zeros for it): the guards there come off. The
/// span to give [`back`] once the call is done, if any came off.
pub(crate) fn lift(at: usize, len: usize) -> Option<(usize, usize)> {
    if HIGH.load(Ordering::Relaxed) == 0 || len == 0 {
        return None;
    }
    let (first, last) = (at >> 16, at.saturating_add(len - 1) >> 16);
    let span = {
        let blocks = BLOCKS.read().unwrap_or_else(|e| e.into_inner());
        let blocks = blocks.as_ref()?;
        let mut hit: Option<(usize, usize)> = None;
        let mut note = |b: usize| hit = Some(hit.map_or((b, b), |(lo, hi)| (lo.min(b), hi.max(b))));
        if last - first < blocks.len() {
            (first..=last).filter(|b| blocks.contains_key(b)).for_each(&mut note);
        } else {
            blocks.keys().copied().filter(|b| (first..=last).contains(b)).for_each(&mut note);
        }
        hit?
    };
    let (from, to) = ((span.0 << 16).max(at & !0xfff), ((span.1 + 1) << 16).min(at.saturating_add(len).saturating_add(0xfff) & !0xfff));
    set_guard(from, to, false).then_some((from, to))
}

/// After [`lift`]: the guards of the watched ranges in the span go back.
pub(crate) fn back(span: Option<(usize, usize)>) {
    let Some((from, to)) = span else {
        return;
    };
    for i in 0..HIGH.load(Ordering::Acquire) {
        let (start, end) = (STARTS[i].load(Ordering::Relaxed), ENDS[i].load(Ordering::Acquire));
        if end != 0 && start < to && from < end {
            set_guard(start.max(from), end.min(to), true);
        }
    }
}

const STATUS_GUARD_PAGE_VIOLATION: i32 = 0x8000_0001_u32 as i32;

/// The handler: first in line; no lock, no allocation.
unsafe extern "system" fn on_guard(info: *mut windows_sys::Win32::System::Diagnostics::Debug::EXCEPTION_POINTERS) -> i32 {
    use windows_sys::Win32::System::Diagnostics::Debug::{EXCEPTION_CONTINUE_EXECUTION, EXCEPTION_CONTINUE_SEARCH};
    // SAFETY: the system passes the exception being dispatched.
    let (r, context) = unsafe { (&*(*info).ExceptionRecord, &*(*info).ContextRecord) };
    if r.ExceptionCode != STATUS_GUARD_PAGE_VIOLATION || r.NumberParameters < 2 {
        return EXCEPTION_CONTINUE_SEARCH;
    }
    let at = r.ExceptionInformation[1];
    let n = HIGH.load(Ordering::Acquire);
    let Some(i) = (0..n).find(|&i| {
        let end = ENDS[i].load(Ordering::Acquire);
        end != 0 && at < ((end + 0xfff) & !0xfff) && at >= (STARTS[i].load(Ordering::Relaxed) & !0xfff)
    }) else {
        return EXCEPTION_CONTINUE_SEARCH;
    };
    let at_ring = (HIT_HEAD.fetch_add(1, Ordering::AcqRel) as usize) & (RING - 1);
    HIT_WHAT[at_ring].store(0, Ordering::Release);
    HIT_CODE[at_ring].store(context.Rip as usize, Ordering::Relaxed);
    HIT_AT[at_ring].store(at, Ordering::Relaxed);
    HIT_WHAT[at_ring].store(NOTED | (r.ExceptionInformation[0] as u64 & 0xff) << 32 | i as u64, Ordering::Release);
    // The system cleared the page's guard; the access goes on.
    EXCEPTION_CONTINUE_EXECUTION
}

/// The handler and the thread that counts hits and puts the guards back,
/// once.
fn install(s: &mut State) {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        images(s);
        tracing::info!(
            level = level(),
            buffers = buffers(),
            other_views = views(),
            title_images = s.images.iter().filter(|i| i.3).count(),
            "read watch: guarding title memory of textures the GPU writes (VICTORY_READ_WATCH)"
        );
        // SAFETY: the handler only reads and writes our atomics.
        unsafe { windows_sys::Win32::System::Diagnostics::Debug::AddVectoredExceptionHandler(1, Some(on_guard)) };
        if cfg!(test) {
            return;
        }
        unsafe extern "C" {
            fn atexit(f: extern "C" fn()) -> i32;
        }
        extern "C" fn at_exit() {
            if let Ok(mut s) = STATE.try_lock()
                && let Some(s) = s.as_mut()
            {
                count(s);
                summary(s, true);
            }
        }
        // SAFETY: a function of ours that takes nothing and returns.
        unsafe { atexit(at_exit) };
        let _ = std::thread::Builder::new().name("victory-read-watch".into()).spawn(|| {
            let interval = std::time::Duration::from_millis(number("VICTORY_READ_WATCH_MS", 1000, 10));
            let every = std::time::Duration::from_secs(number("VICTORY_READ_WATCH_SUMMARY_S", 10, 1));
            let mut last = std::time::Instant::now();
            loop {
                std::thread::sleep(interval);
                tick();
                if last.elapsed() >= every {
                    last = std::time::Instant::now();
                    let mut s = state();
                    summary(s.as_mut().expect("made by state()"), false);
                }
            }
        });
    });
}

/// One round: the hits are counted, slots let go two rounds ago are freed,
/// views that show other pages now are let go, the guards go back.
fn tick() {
    let ranges: Vec<(usize, usize)> = {
        let mut s = state();
        let s = s.as_mut().expect("made by state()");
        count(s);
        s.tick += 1;
        const K64: usize = 64 << 10;
        for i in 0..s.slots.len() {
            let Some(slot) = &s.slots[i] else {
                continue;
            };
            if let Some(dead) = slot.dead {
                if s.tick >= dead + 2 {
                    ENDS[i].store(0, Ordering::Release);
                    STARTS[i].store(0, Ordering::Relaxed);
                    s.slots[i] = None;
                    s.free.push(i);
                }
                continue;
            }
            // Another view: still of the texture's pages?
            if let Some(delta) = slot.view_of {
                let start = STARTS[i].load(Ordering::Relaxed);
                let shown = start.wrapping_sub(delta) & !(K64 - 1);
                if !crate::inplace::pages_of(start & !(K64 - 1)).contains(&shown) {
                    let end = ENDS[i].load(Ordering::Relaxed);
                    let key = slot.key;
                    if let Some(slot) = s.slots[i].as_mut() {
                        slot.dead = Some(s.tick);
                    }
                    if let Some(w) = s.watched.get_mut(&key) {
                        w.slots.retain(|&k| k != i);
                    }
                    if let Some(blocks) = BLOCKS.write().unwrap_or_else(|e| e.into_inner()).as_mut() {
                        for b in start >> 16..=(end - 1) >> 16 {
                            if let Some(n) = blocks.get_mut(&b) {
                                *n -= 1;
                                if *n == 0 {
                                    blocks.remove(&b);
                                }
                            }
                        }
                    }
                    set_guard(start, end, false);
                }
            }
        }
        s.slots
            .iter()
            .enumerate()
            .filter(|(_, slot)| slot.as_ref().is_some_and(|slot| slot.dead.is_none()))
            .map(|(i, _)| (STARTS[i].load(Ordering::Relaxed), ENDS[i].load(Ordering::Relaxed)))
            .collect()
    };
    for (from, to) in ranges {
        // Not while the write guards change protections of their own, if
        // that can be had (`crate::watch`).
        crate::watch::with_guards_if_free(|| set_guard(from, to, true));
    }
}

/// The images loaded, found again when a code address is in none: the
/// title's are those loaded from the image cache.
fn images(s: &mut State) {
    use windows_sys::Win32::System::ProcessStatus::{
        K32EnumProcessModules, K32GetModuleFileNameExW, K32GetModuleInformation, MODULEINFO,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    let mut found = Vec::new();
    // SAFETY: plain queries into buffers of ours.
    unsafe {
        let process = GetCurrentProcess();
        let mut modules = vec![std::ptr::null_mut(); 2048];
        let mut needed = 0;
        if K32EnumProcessModules(process, modules.as_mut_ptr(), (modules.len() * size_of::<usize>()) as u32, &mut needed) == 0 {
            return;
        }
        for &m in &modules[..(needed as usize / size_of::<usize>()).min(modules.len())] {
            let mut name = [0u16; 512];
            let n = K32GetModuleFileNameExW(process, m, name.as_mut_ptr(), name.len() as u32) as usize;
            let name = String::from_utf16_lossy(&name[..n]);
            let mut info: MODULEINFO = std::mem::zeroed();
            if K32GetModuleInformation(process, m, &mut info, size_of::<MODULEINFO>() as u32) != 0 {
                let start = info.lpBaseOfDll as usize;
                let file = name.rsplit('\\').next().unwrap_or("").to_owned();
                let title = file.to_ascii_lowercase().contains("victory-image-");
                found.push((start, start + info.SizeOfImage as usize, file, title));
            }
        }
    }
    #[cfg(test)]
    found.splice(0..0, tests::TITLE.lock().unwrap().iter().cloned());
    s.images = found;
}

/// Counts the hits noted since the last time.
fn count(s: &mut State) {
    let head = HIT_HEAD.load(Ordering::Acquire);
    if head - s.tail > RING as u64 {
        s.lost += head - s.tail - RING as u64;
        s.tail = head - RING as u64;
    }
    let mut looked = false;
    for n in s.tail..head {
        let i = n as usize & (RING - 1);
        let what = HIT_WHAT[i].swap(0, Ordering::AcqRel);
        if what & NOTED == 0 {
            // Being written this moment.
            s.lost += 1;
            continue;
        }
        let (slot, write) = ((what & 0xffff_ffff) as usize, (what >> 32) & 0xff == 1);
        let (code, at) = (HIT_CODE[i].load(Ordering::Relaxed), HIT_AT[i].load(Ordering::Relaxed));
        let Some(Some(Slot { key, view_of, dead: None })) = s.slots.get(slot) else {
            continue;
        };
        let (key, view_of) = (*key, *view_of);
        // The address in the texture: through another view, what it shows.
        let inside = view_of.map_or(at, |delta| at.wrapping_sub(delta));
        if inside < key.0 || inside >= key.0 + key.1 {
            s.beside += 1;
            continue;
        }
        let Some(w) = s.watched.get(&key) else {
            continue;
        };
        let (kind, buffer) = (w.kind, w.buffer);
        if !s.images.iter().any(|&(from, to, ..)| code >= from && code < to) && !looked {
            looked = true;
            images(s);
        }
        let image = s.images.iter().find(|&&(from, to, ..)| code >= from && code < to);
        let name = image.map_or_else(|| format!("{code:#x}"), |(from, _, file, _)| format!("{file}+{:#x}", code - from));
        let title = image.is_some_and(|i| i.3);
        let back = covered(&(kind, buffer));
        let stat = s.stats.entry((kind, buffer)).or_default();
        s.changed = true;
        if !title {
            stat.other += 1;
            *stat.others.entry(name).or_insert(0) += 1;
            continue;
        }
        if view_of.is_some() {
            stat.via_view += 1;
        }
        let counts = stat.codes.entry(name.clone()).or_insert([0; 2]);
        let first = counts[usize::from(write)] == 0;
        counts[usize::from(write)] += 1;
        if write {
            stat.writes += 1;
        } else {
            stat.reads += 1;
            stat.read_textures.insert(key.0);
        }
        if first {
            let kind = kind_text(&kind, buffer);
            let texture = format_args!("{:#x}+{:#x}", key.0, key.1);
            let offset = format_args!("{:#x}", inside - key.0);
            let other_view = view_of.is_some();
            if write {
                tracing::info!(code = name, kind, %texture, %offset, other_view, "read watch: the title's code writes a texture the GPU writes");
            } else {
                tracing::warn!(
                    code = name,
                    kind,
                    %texture,
                    %offset,
                    other_view,
                    back,
                    "read watch: the title's code reads a texture the GPU writes (its bytes in title memory are not what the GPU wrote unless it is copied back)"
                );
            }
        }
    }
    s.tail = head;
}

/// Whether `crate::texback` copies the kind back: "default" (in its default
/// list), "set" (in this run's list only), "no".
fn covered((kind, buffer): &(Kind, bool)) -> &'static str {
    if *buffer {
        return "no";
    }
    let k = (kind[0], kind[1] as u32, kind[4] as u32);
    match (crate::texback::in_default(k), crate::texback::listed(k)) {
        (true, _) => "default",
        (false, true) => "set",
        _ => "no",
    }
}

/// The table: the kinds the title read to the log, all of it to the file.
fn summary(s: &mut State, last: bool) {
    if !s.changed && !last {
        return;
    }
    s.changed = false;
    let live = s.slots.iter().flatten().filter(|slot| slot.dead.is_none()).count();
    let head = format!(
        "read watch: level {} buffers {} | {} ranges guarded now ({} textures) | not watched (table full or too long) {} | hits lost {} | hits beside a range {}",
        level(),
        u8::from(buffers()),
        live,
        s.watched.len(),
        s.full,
        s.lost,
        s.beside
    );
    let mut text = format!(
        "# {head}\n# reads, writes: guard hits from the title's code, at most one per 4 KiB page and interval\n\
         {:<34} {:<26} {:>8} {:>8} {:>9} {:>9} {:>6} {:>8} {:>8}  {}\n",
        "kind", "written as", "textures", "tex_read", "reads", "writes", "codes", "via_view", "other", "texture_back"
    );
    let mut wanted: Vec<String> = Vec::new();
    for (key @ (kind, buffer), stat) in &s.stats {
        let name = kind_text(kind, *buffer);
        let how = stat.how.iter().copied().collect::<Vec<_>>().join("+");
        let codes = stat.codes.values().filter(|c| c[0] != 0).count();
        let back = covered(key);
        text += &format!(
            "{:<34} {:<26} {:>8} {:>8} {:>9} {:>9} {:>6} {:>8} {:>8}  {}\n",
            name,
            how,
            stat.textures.len(),
            stat.read_textures.len(),
            stat.reads,
            stat.writes,
            codes,
            stat.via_view,
            stat.other,
            back
        );
        if stat.reads != 0 {
            if !buffer {
                wanted.push(format!("{}x{}f{}", kind[0], kind[1], kind[4]));
            }
            tracing::info!(
                kind = name,
                written_as = how,
                textures = stat.textures.len(),
                textures_read = stat.read_textures.len(),
                reads = stat.reads,
                writes = stat.writes,
                codes,
                via_view = stat.via_view,
                other = stat.other,
                texture_back = back,
                "read watch: a kind the title's code reads"
            );
        }
    }
    let mut list: Vec<&str> = crate::texback::DEFAULT.split(',').collect();
    for w in &wanted {
        if !list.contains(&w.as_str()) {
            list.push(w);
        }
    }
    let list = list.join(",");
    tracing::info!(kinds = s.stats.len(), kinds_read = wanted.len(), texture_back = list, "{head}");
    let Some(file) = std::env::var_os("VICTORY_READ_WATCH_FILE") else {
        return;
    };
    text += &format!("# every kind read, with the default ones: VICTORY_TEXTURE_BACK={list}\n\n# code addresses per kind (reads, writes); then code that is not the title's (hits)\n");
    for ((kind, buffer), stat) in &s.stats {
        if stat.codes.is_empty() && stat.others.is_empty() {
            continue;
        }
        text += &format!("{}\n", kind_text(kind, *buffer));
        for (code, [reads, writes]) in &stat.codes {
            text += &format!("    {code:<48} reads {reads:>8} writes {writes:>8}\n");
        }
        for (code, hits) in &stat.others {
            text += &format!("    other {code:<42} hits  {hits:>8}\n");
        }
    }
    if let Err(e) = std::fs::write(&file, text) {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| tracing::warn!(error = %e, "read watch: its file could not be written (VICTORY_READ_WATCH_FILE)"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::System::Memory::{
        MEM_COMMIT, MEM_RESERVE, MEMORY_BASIC_INFORMATION, PAGE_GUARD, PAGE_READWRITE, VirtualAlloc, VirtualQuery,
    };

    /// Images the test calls the title's.
    pub(super) static TITLE: Mutex<Vec<(usize, usize, String, bool)>> = Mutex::new(Vec::new());

    #[inline(never)]
    fn title_reads(at: *const u8) -> u8 {
        // SAFETY: the test's own page.
        unsafe { at.read_volatile() }
    }

    #[inline(never)]
    fn title_writes(at: *mut u8) {
        // SAFETY: the test's own page.
        unsafe { at.write_volatile(7) }
    }

    fn guarded(at: usize) -> bool {
        // SAFETY: plain data filled in by the query.
        let mut info: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: a valid out pointer of the right size.
        unsafe { VirtualQuery(at as *const _, &mut info, size_of_val(&info)) };
        info.Protect & PAGE_GUARD != 0
    }

    /// A read and a write from code called the title's are counted for the
    /// texture's kind, each once per page and round; the host's kernel copy
    /// reads through the guards; a released texture's guards come off.
    #[test]
    fn hits_are_counted_per_kind_and_guards_come_off() {
        // The code of this test binary stands for the title's.
        let here = title_reads as *const () as usize;
        TITLE.lock().unwrap().push((here & !0xffff_ffff, (here & !0xffff_ffff) + (1 << 33), "title".into(), true));
        const LEN: usize = 8 * 4096;
        // SAFETY: a fresh allocation, kept.
        let at = unsafe { VirtualAlloc(std::ptr::null(), LEN, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE) } as usize;
        assert_ne!(at, 0);
        let kind: Kind = [64, 32, 1, 1, 41];
        watch(at, LEN, kind, false, "unordered", 0x1234);
        assert!(guarded(at) && guarded(at + LEN - 1));
        assert_eq!(title_reads(at as *const u8), 0);
        assert_eq!(title_reads(at as *const u8), 0);
        title_writes((at + 4096) as *mut u8);
        assert!(!guarded(at) && guarded(at + 2 * 4096));
        // The host's copy through the kernel: all of it, guards or not.
        let mut copy = vec![1u8; LEN];
        assert!(crate::host::read_into(at as *const u8, &mut copy));
        assert_eq!(copy[4096], 7);
        assert!(guarded(at + 2 * 4096), "the guards go back after the host's copy");
        tick();
        assert!(guarded(at), "a round puts the guards back");
        {
            let s = state();
            let stat = &s.as_ref().unwrap().stats[&(kind, false)];
            assert_eq!((stat.reads, stat.writes, stat.textures.len(), stat.read_textures.len()), (1, 1, 1, 1));
            assert_eq!(stat.codes.len(), 2);
            assert_eq!(stat.other, 0);
        }
        dropped(0x1234 as This);
        assert!(!guarded(at) && !guarded(at + 2 * 4096));
        // A touch after the release is nobody's.
        assert_eq!(title_reads((at + 2 * 4096) as *const u8), 0);
        tick();
        tick();
        tick();
        let mut s = state();
        let s = s.as_mut().unwrap();
        assert!(s.watched.is_empty() && s.free.len() == 1);
        assert_eq!(s.stats[&(kind, false)].reads, 1);
        summary(s, true);
    }

    #[test]
    fn a_buffer_views_bytes() {
        // 16 elements of 12 bytes from element 4.
        let desc = [0, 1, 4, 0, 16, 12, 0, 0, 0, 0];
        assert_eq!(buffer_range(Some(&desc), 4096), (48, 192, 12, 0));
        // Raw: words.
        let raw = [39, 1, 2, 0, 8, 0, 0, 0, 1, 0];
        assert_eq!(buffer_range(Some(&raw), 4096), (8, 32, 4, 39));
        assert_eq!(buffer_range(None, 4096), (0, 4096, 0, 0));
    }
}
