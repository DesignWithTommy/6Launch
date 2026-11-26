//! Which title memory the title wrote since a host copy took it (decision
//! 0007, "Copies of title memory"). The runtime reserves title GPU memory
//! with write watching (`victory-hle` xmem), so the kernel keeps a dirty bit
//! per 4 KiB page; asking for a range's written pages also clears them. Host
//! copies of the same memory can overlap (nested buffers, textures in
//! buffers), so what one asks is kept here for the others: a journal of
//! each page's last write, numbered by the asking.
//!
//! Physical-page views (xmem's sections) cannot be write-watched. There a
//! copy leaves each 64 KiB block it took read-only, and our vectored handler
//! takes the first write to it: it notes the block as written and gives the
//! write access back. The runtime tells us when it maps or unmaps views
//! ([`title_memory_remapped`]), which resets their protection. Blocks found
//! wholly uncommitted are skipped while they remain so; one query can
//! confirm a whole region, including ordinary reservations between views.
//!
//! Writes into physical pages through another view are seen too: the
//! runtime tells us the views of the pages it wrote itself ([`writing`],
//! xmem's `write_physical` and DirectStorage's page destinations), and a
//! write by the title through any view of a guarded page faults like one
//! at the guarded address (the guard makes every view of the page
//! read-only, [`open`] gives them all back; not a page of more than
//! [`MANY_VIEWS`] views). A
//! system call writing into a guarded block fails (none is known to).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

/// The unit tracked and copied: 64 KiB, the console's page (the kernel's
/// 4 KiB pages fold into it). Finer units made the checks of the world's
/// big buffers cost more than the copies they saved.
pub(crate) const PAGE: usize = 64 << 10;
/// The kernel's page, the unit of its answers.
const KERNEL_PAGE: usize = 4096;

// 4 KiB pieces of a page, one bit each
pub(crate) const PIECE_BYTES: usize = KERNEL_PAGE;
const PIECES: usize = PAGE / KERNEL_PAGE;
const _: () = assert!(PIECES == 16, "a page's pieces are the bits of a u16");

// refreshes copy the written 4 KiB pieces of a page, not the whole 64 KiB (VICTORY_FINE_PAGES=1)
pub(crate) fn fine_pages() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_FINE_PAGES").is_some_and(|v| v == "1"))
}

// pieces written at `last`; every other piece last written at or before `older`
#[derive(Clone, Copy)]
pub(crate) struct Pieces {
    pub(crate) last: u64,
    pub(crate) older: u64,
    pub(crate) mask: u16,
}

/// A title-memory range whose changed pages another subsystem needs to read.
pub(crate) struct Tracked {
    start: usize,
    end: usize,
    seen: Mutex<BTreeMap<usize, u64>>,
}

/// Tracks a runtime GPU-memory range, including physical-page views when
/// `views` is true. Other process memory is not tracked.
pub(crate) fn track(start: usize, len: usize, views: bool) -> Option<Tracked> {
    let end = start.checked_add(len)?;
    if len == 0 || writes(start, end, views).is_none() {
        return None;
    }
    Some(Tracked {
        start,
        end,
        seen: Mutex::default(),
    })
}

struct Journal {
    /// Numbers the askings; 0 is "never".
    asked: u64,
    /// Page number → the asking that last found it written.
    written: Pages,
    /// Room for the kernel's answer.
    found: Vec<usize>,
    /// Page ranges asked ahead for a submission's refreshes ([`ask_ahead`])
    /// and the asking that did: `writes` answers for a range inside one
    /// from the journal, without another kernel call.
    ahead: Vec<(usize, usize)>,
    ahead_asked: u64,
    /// The ahead asking stands for a whole submission ([`ask_once`]): no
    /// other ahead asking is made, and `ahead` stays, until
    /// [`ask_once_end`].
    kept: bool,
    // with fine_pages: which pieces of each page the last asking found written
    pieces: Pages<Pieces>,
    // with ask_lean: first pages of ranges the kernel turned down lately, and pre-askings since cleared
    refused: BTreeSet<usize>,
    refused_age: u32,
}

// a submission's pre-asking leaves out view memory and ranges the kernel turned down lately (VICTORY_ASK_LEAN=1)
pub(crate) fn ask_lean() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_ASK_LEAN").is_some_and(|v| v == "1"))
}

// pre-askings between clearings of the turned-down ranges
const REFUSED_AGE: u32 = 512;

// ranges left out of pre-askings since the frame line: guarded views, turned down lately
pub(crate) static LEAN_SKIPPED: [std::sync::atomic::AtomicU64; 2] =
    [const { std::sync::atomic::AtomicU64::new(0) }; 2];

// the ranges worth a pre-asking: a skipped one is asked by `writes` at its refresh, as before
fn lean_ranges(ranges: &[(usize, usize)]) -> Vec<(usize, usize)> {
    use std::sync::atomic::Ordering::Relaxed;
    let refused: Vec<bool> = {
        let mut j = JOURNAL.lock().unwrap_or_else(|e| e.into_inner());
        j.refused_age += 1;
        if j.refused_age >= REFUSED_AGE {
            j.refused_age = 0;
            j.refused.clear();
        }
        ranges.iter().map(|r| j.refused.contains(&(r.0 / PAGE))).collect()
    };
    let mut out = Vec::with_capacity(ranges.len());
    for (&(start, end), refused) in ranges.iter().zip(refused) {
        if start >= end {
            continue;
        }
        if refused {
            LEAN_SKIPPED[1].fetch_add(1, Relaxed);
        } else if known_view(start) {
            LEAN_SKIPPED[0].fetch_add(1, Relaxed);
        } else {
            out.push((start, end));
        }
    }
    out
}

// notes `page` written at `now` in the pieces of `mask`
fn note(j: &mut Journal, page: usize, now: u64, mask: u16) {
    crate::push::changed(page);
    let prev = j.written.insert(page, now);
    if !fine_pages() {
        return;
    }
    let prev = prev.unwrap_or(0);
    let e = j.pieces.entry(page, Pieces { last: prev, older: prev, mask: u16::MAX });
    if e.last == now {
        e.mask |= mask;
    } else {
        // what was written before stays bounded by the old asking
        e.older = e.last;
        e.last = now;
        e.mask = mask;
    }
}

// notes the kernel's answer (4 KiB page addresses, in order) at `now`, a page at a time
fn note_found(j: &mut Journal, count: usize, now: u64) {
    let mut k = 0;
    while k < count {
        let page = j.found[k] / PAGE;
        let mut mask = 0u16;
        while k < count && j.found[k] / PAGE == page {
            mask |= 1 << ((j.found[k] % PAGE) / KERNEL_PAGE);
            k += 1;
        }
        note(j, page, now, mask);
    }
}

// the bits of the pieces of `page` that `[from, to)` touches
fn piece_mask(page: usize, from: usize, to: usize) -> u16 {
    let (a, b) = (from.max(page * PAGE), to.min((page + 1) * PAGE));
    if a >= b {
        return 0;
    }
    let (first, last) = ((a - page * PAGE) / KERNEL_PAGE, (b - page * PAGE).div_ceil(KERNEL_PAGE));
    ((((1u32 << last) - 1) & !((1u32 << first) - 1)) & 0xffff) as u16
}

/// The pieces noted for the pages of `[start, end)` (whole pages), in page
/// order; empty without [`fine_pages`].
pub(crate) fn pieces(start: usize, end: usize) -> Vec<(usize, Pieces)> {
    if !fine_pages() {
        return Vec::new();
    }
    let (first, last) = (start / PAGE, end.div_ceil(PAGE));
    let mut j = JOURNAL.lock().unwrap_or_else(|e| e.into_inner());
    j.pieces.range(first, last)
}

/// The journal's 4 KiB piece notes kept flat as its pages are
/// (`VICTORY_PIECES_FLAT=1`; off by default; with `VICTORY_FINE_PAGES=1`):
/// each refresh's [`pieces`] is a slice copy, not a tree walk collected.
fn pieces_flat() -> bool {
    // live: the notes change form under the journal's lock at the next use
    static S: crate::switch::Switch =
        crate::switch::Switch::noted("PIECES_FLAT", "the write journal's piece notes are kept flat (VICTORY_PIECES_FLAT=1)");
    S.on()
}

// the pieces of a page a copy stamped `since` lacks, given the page's last write and its noted pieces
pub(crate) fn pieces_needed(since: u64, write: u64, noted: Option<Pieces>) -> u16 {
    match noted {
        Some(n) if since != 0 && n.last == write && n.older <= since => n.mask,
        _ => u16::MAX,
    }
}

static JOURNAL: Mutex<Journal> = Mutex::new(Journal {
    asked: 0,
    written: Pages::new(journal_flat),
    found: Vec::new(),
    ahead: Vec::new(),
    ahead_asked: 0,
    kept: false,
    pieces: Pages::new(pieces_flat),
    refused: BTreeSet::new(),
    refused_age: 0,
});

/// Ranges of one submission closer than this are asked about together:
/// walking the pages between them costs less than another kernel call (a
/// call that resets written pages costs 30-70 microseconds whatever the
/// range).
const AHEAD_GAP: usize = (8 << 20) / PAGE;
/// The most one asking covers (its answer needs 8 bytes per 4 KiB page).
const AHEAD_MAX: usize = (1 << 30) / PAGE;

/// Asks the kernel about many ranges `(start, end)` at once, before the
/// [`writes`] of each: ranges close together are one asking, and `writes`
/// then answers from the journal until [`ask_ahead_done`]. The world's
/// submissions refresh thousands of host copies (6,664 askings a frame,
/// 456 ms, in run 14). Ranges that are not write-watched, or lie alone, are
/// left to `writes`.
pub(crate) fn ask_ahead(ranges: &[(usize, usize)]) {
    let mut pieces: Vec<(usize, usize)> = ranges
        .iter()
        .filter(|r| r.0 < r.1)
        .map(|&(start, end)| (start / PAGE, end.div_ceil(PAGE)))
        .collect();
    if pieces.len() < 2 {
        return;
    }
    pieces.sort_unstable();
    let mut held = JOURNAL.lock().unwrap_or_else(|e| e.into_inner());
    let j = &mut *held;
    // The submission's one asking stands ([`ask_once`]): what it does not
    // hold is left to `writes`.
    if j.kept {
        return;
    }
    // View memory the guards hold is left to `writes`, which answers it
    // from the guards ([`known_view`]): the kernel's write watch can only
    // fail there. Asked here with the others, a run holding such a range
    // failed, was asked again by halves down to single ranges, failing
    // each time, and the range then went to `writes` all the same (after
    // triippywhale d0581a55). `VICTORY_VIEW_ASK=1` asks as before.
    #[cfg(feature = "diagnostics")]
    let all = pieces.len();
    pieces.retain(|&(first, _)| !known_view(first * PAGE));
    #[cfg(feature = "diagnostics")]
    AHEAD_VIEWS.fetch_add((all - pieces.len()) as u64, std::sync::atomic::Ordering::Relaxed);
    j.asked += 1;
    j.ahead_asked = j.asked;
    j.ahead.clear();
    if wide_asks() {
        let now = j.ahead_asked;
        drop(held);
        return asked_wide(&pieces, now);
    }
    for run in ahead_runs(&pieces) {
        ask_run(j, run);
    }
}

/// Whether [`ask_ahead`]'s kernel askings run on the wide threads
/// (`VICTORY_WIDE_ASKS=1`; off by default), the journal's lock let go
/// meanwhile, their answers then noted in order under the same asking.
///
/// Why: in the prof-s-1 profile 3.8 ms a frame of the submitting thread
/// were these askings (`NtGetWriteWatch`) one after another, at each
/// submission's start.
fn wide_asks() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_WIDE_ASKS").is_some_and(|v| v == "1"))
}

/// [`ask_ahead`]'s runs of `pieces` (page ranges in order), as it makes
/// them.
fn ahead_runs(pieces: &[(usize, usize)]) -> Vec<&[(usize, usize)]> {
    let mut runs = Vec::new();
    let mut i = 0;
    while i < pieces.len() {
        let first = pieces[i].0;
        let mut last = pieces[i].1;
        let mut k = i + 1;
        while k < pieces.len()
            && pieces[k].0 <= last + AHEAD_GAP
            && last.max(pieces[k].1) - first <= AHEAD_MAX
        {
            last = last.max(pieces[k].1);
            k += 1;
        }
        runs.push(&pieces[i..k]);
        i = k;
    }
    runs
}

/// With [`wide_asks`]: [`ask_ahead`]'s askings of `pieces` on the wide
/// threads, without the journal; noted at asking `now` in order after.
fn asked_wide(pieces: &[(usize, usize)], now: u64) {
    type Answers = Vec<(usize, usize, Vec<usize>)>;
    let runs: Vec<Vec<(usize, usize)>> = ahead_runs(pieces)
        .into_iter()
        .filter(|r| r.len() >= 2)
        .map(<[_]>::to_vec)
        .collect();
    let answers: std::sync::Arc<Mutex<Vec<Answers>>> =
        std::sync::Arc::new(Mutex::new(vec![Vec::new(); runs.len()]));
    let jobs: Vec<Box<dyn FnOnce() + Send>> = runs
        .into_iter()
        .enumerate()
        .map(|(k, run)| {
            let answers = answers.clone();
            Box::new(move || {
                let mut found = Vec::new();
                run_alone(&run, &mut found);
                answers.lock().unwrap_or_else(|e| e.into_inner())[k] = found;
            }) as Box<dyn FnOnce() + Send>
        })
        .collect();
    crate::shadow::wide_run(jobs);
    let answers = std::mem::take(&mut *answers.lock().unwrap_or_else(|e| e.into_inner()));
    let mut j = JOURNAL.lock().unwrap_or_else(|e| e.into_inner());
    for (first, last, written) in answers.into_iter().flatten() {
        note_addresses(&mut j, &written, now);
        j.ahead.push((first, last));
    }
}

/// [`ask_run`] without the journal: the runs the kernel answered, with
/// the written 4 KiB page addresses it gave, in order.
fn run_alone(pieces: &[(usize, usize)], out: &mut Vec<(usize, usize, Vec<usize>)>) {
    if pieces.len() < 2 {
        return;
    }
    let first = pieces[0].0;
    let last = pieces.iter().map(|p| p.1).max().unwrap_or(first);
    if let Some(written) = kernel_written(first, last) {
        out.push((first, last, written));
        return;
    }
    let (a, b) = pieces.split_at(pieces.len() / 2);
    run_alone(a, out);
    run_alone(b, out);
}

/// The kernel's written 4 KiB page addresses of pages `[first, last)`,
/// reset; `None` if the range is not write-watched memory of one
/// reservation.
fn kernel_written(first: usize, last: usize) -> Option<Vec<usize>> {
    use windows_sys::Win32::System::Memory::GetWriteWatch;
    const WRITE_WATCH_FLAG_RESET: u32 = 1;
    let mut count = (last - first) * PAGE / KERNEL_PAGE;
    let mut found = vec![0usize; count];
    let mut granularity = 0;
    // SAFETY: the answer buffer holds `count` entries.
    let failed = unsafe {
        GetWriteWatch(
            WRITE_WATCH_FLAG_RESET,
            (first * PAGE) as *const _,
            (last - first) * PAGE,
            found.as_mut_ptr().cast(),
            &mut count,
            &mut granularity,
        )
    } != 0;
    if failed {
        return None;
    }
    found.truncate(count);
    Some(found)
}

/// [`note_found`] of `addresses` (4 KiB page addresses, in order).
fn note_addresses(j: &mut Journal, addresses: &[usize], now: u64) {
    let mut k = 0;
    while k < addresses.len() {
        let page = addresses[k] / PAGE;
        let mut mask = 0u16;
        while k < addresses.len() && addresses[k] / PAGE == page {
            mask |= 1 << ((addresses[k] % PAGE) / KERNEL_PAGE);
            k += 1;
        }
        note(j, page, now, mask);
    }
}

/// Since the last frame line: the kernel askings that took a millisecond
/// or more, their microseconds, and the longest asking's. An asking walks
/// page tables, some ten microseconds for most ranges; in run 75 the 2,644
/// askings of a frame took 219 ms in the lines where the pass over the
/// heap's views ran long, 25-50 ms in the others. These numbers say
/// whether a few askings wait for something or all of them are slow.
#[cfg(feature = "diagnostics")]
static SLOW: [std::sync::atomic::AtomicU64; 3] = [const { std::sync::atomic::AtomicU64::new(0) }; 3];

#[cfg(feature = "diagnostics")]
fn slow(us: u64) {
    use std::sync::atomic::Ordering::Relaxed;
    if us >= 1000 {
        SLOW[0].fetch_add(1, Relaxed);
        SLOW[1].fetch_add(us, Relaxed);
    }
    SLOW[2].fetch_max(us, Relaxed);
}

/// Since the last frame line: ranges of view memory the guards hold that a
/// submission's askings ahead ([`ask_ahead`], [`ask_once`]) left out. Each
/// was a kernel asking that failed, or several, before.
#[cfg(feature = "diagnostics")]
pub(crate) static AHEAD_VIEWS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// [`SLOW`], taken.
#[cfg(feature = "diagnostics")]
pub(crate) fn slow_asks() -> [u64; 3] {
    [0, 1, 2].map(|i| SLOW[i].swap(0, std::sync::atomic::Ordering::Relaxed))
}

/// The submission's refreshes are done: `writes` asks the kernel again.
pub(crate) fn ask_ahead_done() {
    let mut j = JOURNAL.lock().unwrap_or_else(|e| e.into_inner());
    if !j.kept {
        j.ahead.clear();
    }
}

/// Whether a submission asks the kernel once, before its first refresh,
/// about every range its lists refresh (`VICTORY_ASK_ONCE=1`; off by
/// default).
///
/// A submission's refreshes are done in groups, between the title's own
/// buffer copies (each copy flushes the refreshes recorded before it), and
/// every group asked the kernel again: 2,644 askings a frame, 70 a
/// submission. With the switch the journal answers all of them from one
/// asking made when the submission starts.
///
/// What changes: memory the title's other threads write while the
/// submission is being prepared is seen by the next submission's asking,
/// not by this one's later groups. What this submission's lists read was
/// written before they were submitted, so before the asking. Our own
/// writes into title memory while preparing are told to the journal
/// ([`wrote`]) or end the asking ([`ask_once_end`]).
pub(crate) fn once() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    // On by default since run 81 (with card buffers: 375-450 ms a frame
    // against 573-647, the picture unchanged over four minutes);
    // `VICTORY_ASK_ONCE=0` asks per range, as before.
    *ON.get_or_init(|| std::env::var_os("VICTORY_ASK_ONCE").is_none_or(|v| v != "0"))
}

/// With [`once`]: asks about `ranges` (everything a submission's lists
/// refresh) as [`ask_ahead`] does, and lets that asking stand until
/// [`ask_once_end`].
pub(crate) fn ask_once(ranges: &[(usize, usize)]) {
    if !once() {
        return;
    }
    ask_once_end();
    let lean = ask_lean().then(|| lean_ranges(ranges));
    let ranges = lean.as_deref().unwrap_or(ranges);
    ask_ahead(ranges);
    let mut j = JOURNAL.lock().unwrap_or_else(|e| e.into_inner());
    let j = &mut *j;
    if j.ahead.is_empty() {
        // `ask_ahead` asked nothing (fewer than two ranges).
        j.asked += 1;
        j.ahead_asked = j.asked;
    }
    // `ask_ahead` leaves a range that lies alone to `writes`, which asks
    // once per flush: with one asking for the submission that was every
    // such range asked again at each of its 70 flushes (run 76: 2,300
    // askings a frame with the switch on, as without it). They are asked
    // here, once; what is not write-watched stays with `writes`.
    for &(start, end) in ranges {
        if start >= end {
            continue;
        }
        // View memory the guards hold is not asked about: the asking can
        // only fail there, one failed kernel call for each such copy at
        // every submission, and `writes` answers it from the guards
        // (after triippywhale cc368c7c).
        if known_view(start) {
            #[cfg(feature = "diagnostics")]
            AHEAD_VIEWS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            continue;
        }
        let (first, last) = (start / PAGE, end.div_ceil(PAGE));
        if !j.ahead.iter().any(|&(a, b)| a <= first && last <= b) {
            if ask_kernel(j, first, last) {
                j.ahead.push((first, last));
            } else if ask_lean() {
                j.refused.insert(first);
            }
        }
    }
    j.kept = !j.ahead.is_empty();
}

/// The submission's one asking stands no longer: its refreshes are done,
/// or title memory was written in a way the journal was not told.
pub(crate) fn ask_once_end() {
    let mut j = JOURNAL.lock().unwrap_or_else(|e| e.into_inner());
    if j.kept {
        j.kept = false;
        j.ahead.clear();
    }
}

/// We wrote title memory `[at, at + len)` while a submission's one asking
/// stands (the title's buffer copies, done at submission): its pages count
/// as written after that asking, so the refreshes that follow copy them.
pub(crate) fn wrote(at: usize, len: usize) {
    let mut j = JOURNAL.lock().unwrap_or_else(|e| e.into_inner());
    if !j.kept || len == 0 {
        return;
    }
    j.asked += 1;
    let now = j.asked;
    // The asking that stands moves with it: `asked` and `writes` give it as
    // "now", and a page noted written after "now" stays stale for every
    // copy that takes it (copied at `now`, written at `now + 1`): copied
    // again at every refresh, and a reason to settle flights
    // (fxceless/victory-fork#410).
    j.ahead_asked = now;
    let end = at.saturating_add(len);
    for page in at / PAGE..end.div_ceil(PAGE) {
        note(&mut j, page, now, piece_mask(page, at, end));
    }
}

/// We wrote the 64 KiB page numbered `page` of title memory through a
/// view of our own (`crate::inplace::write_own`): no guard was opened and
/// no kernel write-watch bit set, so the journal is told here. The page,
/// and the same physical page at its other title addresses (`others`,
/// page numbers), count as written at a new asking, which every host copy
/// that holds them takes at its next refresh. Returns the asking that had
/// last found `page` written (0: never) and the new one, for the copy
/// that wrote to note itself current.
pub(crate) fn wrote_own(page: usize, others: &[usize]) -> (u64, u64) {
    let mut j = JOURNAL.lock().unwrap_or_else(|e| e.into_inner());
    let j = &mut *j;
    j.asked += 1;
    let now = j.asked;
    // As in [`wrote`]: an asking that stands for a submission, or for a
    // flush, moves with it, or a copy taking the page at the standing
    // asking would find it written "later" at every refresh.
    if j.kept || !j.ahead.is_empty() {
        j.ahead_asked = now;
    }
    let before = j.written.insert(page, now).unwrap_or(0);
    crate::push::changed(page);
    for &other in others {
        if other != page {
            j.written.insert(other, now);
            crate::push::changed(other);
        }
    }
    (before, now)
}

/// One kernel asking for `pieces` (page ranges in order); by halves if it
/// fails, as a run across two reservations or into memory that is not
/// write-watched does. A piece alone is left to [`writes`].
fn ask_run(j: &mut Journal, pieces: &[(usize, usize)]) {
    if pieces.len() < 2 {
        return;
    }
    let first = pieces[0].0;
    let last = pieces.iter().map(|p| p.1).max().unwrap_or(first);
    if ask_kernel(j, first, last) {
        j.ahead.push((first, last));
        return;
    }
    let (a, b) = pieces.split_at(pieces.len() / 2);
    ask_run(j, a);
    ask_run(j, b);
}

/// Asks the kernel for the written pages of pages `[first, last)` and notes
/// them in the journal under the ahead asking; false (and nothing changed)
/// if the range is not write-watched memory of one reservation.
fn ask_kernel(j: &mut Journal, first: usize, last: usize) -> bool {
    use windows_sys::Win32::System::Memory::GetWriteWatch;
    const WRITE_WATCH_FLAG_RESET: u32 = 1;
    let mut count = (last - first) * PAGE / KERNEL_PAGE;
    j.found.resize(count.max(j.found.len()), 0);
    let mut granularity = 0;
    #[cfg(feature = "diagnostics")]
    let started = std::time::Instant::now();
    // SAFETY: the answer buffer holds `count` entries.
    let failed = unsafe {
        GetWriteWatch(
            WRITE_WATCH_FLAG_RESET,
            (first * PAGE) as *const _,
            (last - first) * PAGE,
            j.found.as_mut_ptr().cast(),
            &mut count,
            &mut granularity,
        )
    } != 0;
    if failed {
        return false;
    }
    #[cfg(feature = "diagnostics")]
    {
        use crate::copystat::{C, add};
        add(C::Watches, 1);
        add(C::WatchUs, started.elapsed().as_micros() as u64);
        slow(started.elapsed().as_micros() as u64);
    }
    let now = j.ahead_asked;
    note_found(j, count, now);
    true
}

/// What [`writes`] would say of `[start, end)` from the journal alone, if
/// the range lies in one asked ahead ([`ask_ahead`], [`ask_once`]): the
/// pages written since tracking began and the asking that last found each
/// written, with the number of the asking that stands (as [`writes`] gives
/// it). `None` if the kernel would have to be asked.
pub(crate) fn asked(start: usize, end: usize) -> Option<(u64, Vec<(usize, u64)>)> {
    let (first, last) = (start / PAGE, end.div_ceil(PAGE));
    let mut j = JOURNAL.lock().unwrap_or_else(|e| e.into_inner());
    let j = &mut *j;
    j.ahead
        .iter()
        .any(|&(a, b)| a <= first && last <= b)
        .then(|| (j.ahead_asked, j.written.range(first, last)))
}

// push: whether writes() would answer `[start, end)` now with no question to the kernel or to Windows
pub(crate) fn standing(start: usize, end: usize) -> bool {
    let (first, last) = (start / PAGE, end.div_ceil(PAGE));
    if JOURNAL.lock().unwrap_or_else(|e| e.into_inner()).ahead.iter().any(|&(a, b)| a <= first && last <= b) {
        return true;
    }
    // absent blocks are asked about at every pass, or with commit notices on retry frames
    let asks_absent = !crate::guard_log::commit_notice() || crate::guard_log::retry_frame(crate::frame::current());
    let g = guards();
    let known = g.protected.contains(&first)
        || g.absent.contains(&first)
        || ((seen_views() || view_known()) && g.views.contains(&first));
    known && !(asks_absent && g.absent.range(first..last).next().is_some())
}

/// The pages of `[start, end)` (whole pages) as last written, for pages
/// written since tracking began: `(page number, asking)` in page order, and
/// this asking's number, which is higher than any in the list but those the
/// kernel just reported. `None` if the memory is not write-watched (nor,
/// with `guarded`, view memory, which is then guarded).
pub(crate) fn writes(start: usize, end: usize, guarded: bool) -> Option<(u64, Vec<(usize, u64)>)> {
    use windows_sys::Win32::System::Memory::GetWriteWatch;
    const WRITE_WATCH_FLAG_RESET: u32 = 1;
    let (first, last) = (start / PAGE, end.div_ceil(PAGE));
    let mut j = JOURNAL.lock().unwrap_or_else(|e| e.into_inner());
    let j = &mut *j;
    // Asked ahead with the submission's other ranges: the journal has it.
    if j.ahead.iter().any(|&(a, b)| a <= first && last <= b) {
        return Some((j.ahead_asked, j.written.range(first, last)));
    }
    let mut count = (last - first) * PAGE / KERNEL_PAGE;
    j.found.resize(count.max(j.found.len()), 0);
    let mut granularity = 0;
    #[cfg(feature = "diagnostics")]
    let started = std::time::Instant::now();
    // View memory the guards already hold cannot be write-watched: the
    // kernel asking would only fail. Copies of it are flushed many times a
    // frame, each time paying that failed call before the guards answer.
    let known_view = guarded && known_view(first * PAGE);
    // SAFETY: the answer buffer holds `count` entries.
    let failed = known_view
        || unsafe {
            GetWriteWatch(
                WRITE_WATCH_FLAG_RESET,
                (first * PAGE) as *const _,
                (last - first) * PAGE,
                j.found.as_mut_ptr().cast(),
                &mut count,
                &mut granularity,
            )
        } != 0;
    #[cfg(feature = "diagnostics")]
    crate::copystat::add(
        if known_view {
            crate::copystat::C::WatchKnownView
        } else {
            crate::copystat::C::WatchFailed
        },
        failed as u64,
    );
    #[cfg(feature = "diagnostics")]
    if failed && !known_view && guarded {
        failed_asking(first * PAGE, last * PAGE);
    }
    let guarded = if failed {
        count = 0;
        if !guarded {
            return None;
        }
        #[cfg(feature = "diagnostics")]
        let guarding = std::time::Instant::now();
        let written = guard(first * PAGE, last * PAGE);
        #[cfg(feature = "diagnostics")]
        crate::copystat::add(crate::copystat::C::GuardUs, guarding.elapsed().as_micros() as u64);
        written?
    } else {
        Vec::new()
    };
    #[cfg(feature = "diagnostics")]
    {
        use crate::copystat::{C, add};
        add(C::Watches, 1);
        add(C::WatchUs, started.elapsed().as_micros() as u64);
        slow(started.elapsed().as_micros() as u64);
    }
    j.asked += 1;
    let now = j.asked;
    note_found(j, count, now);
    // guarded blocks are known by the 64 KiB only
    for page in guarded {
        note(j, page, now, u16::MAX);
    }
    Some((now, j.written.range(first, last)))
}

/// The journal's pages kept flat (`VICTORY_JOURNAL_FLAT=1`; off by default):
/// a sorted vec, so a refresh's range is one slice copy, not a tree walk
/// collected entry by entry (s2-prep-1: 16.5 ms a frame in `writes`' collects).
/// Pages new to the journal wait in a small tree and are merged in at the
/// next range read.
fn journal_flat() -> bool {
    // live: the pages change form under the journal's lock at the next use
    static S: crate::switch::Switch =
        crate::switch::Switch::noted("JOURNAL_FLAT", "the write journal's pages are kept flat (VICTORY_JOURNAL_FLAT=1)");
    S.on()
}

/// New pages that wait before the flat journal is merged whole.
const FRESH_MOST: usize = 1024;

/// Page number → the asking that last found it written; a tree, or with
/// [`journal_flat`] a sorted vec and the pages not merged into it yet.
struct Pages<V: Copy = u64> {
    tree: BTreeMap<usize, V>,
    flat: Vec<(usize, V)>,
    fresh: BTreeMap<usize, V>,
    // 0 follows the switch, 1 tree, 2 flat (tests set it)
    mode: u8,
    // the form the pages are in now
    flat_now: bool,
    // the switch that keeps this one flat
    switch: fn() -> bool,
}

impl<V: Copy> Pages<V> {
    const fn new(switch: fn() -> bool) -> Self {
        Pages {
            tree: BTreeMap::new(),
            flat: Vec::new(),
            fresh: BTreeMap::new(),
            mode: 0,
            flat_now: false,
            switch,
        }
    }

    fn is_flat(&mut self) -> bool {
        let want = match self.mode {
            0 => (self.switch)(),
            m => m == 2,
        };
        if want != self.flat_now {
            self.reform(want);
        }
        want
    }

    // the switch flipped: every page moves to the other form
    fn reform(&mut self, flat: bool) {
        if flat {
            self.flat = std::mem::take(&mut self.tree).into_iter().collect();
        } else {
            self.merge();
            self.tree = std::mem::take(&mut self.flat).into_iter().collect();
        }
        self.flat_now = flat;
    }

    fn insert(&mut self, page: usize, now: V) -> Option<V> {
        if !self.is_flat() {
            return self.tree.insert(page, now);
        }
        match self.flat.binary_search_by_key(&page, |e| e.0) {
            Ok(i) => Some(std::mem::replace(&mut self.flat[i].1, now)),
            Err(_) => self.fresh.insert(page, now),
        }
    }

    /// The page's value, `new` put in first if it has none.
    fn entry(&mut self, page: usize, new: V) -> &mut V {
        if !self.is_flat() {
            return self.tree.entry(page).or_insert(new);
        }
        match self.flat.binary_search_by_key(&page, |e| e.0) {
            Ok(i) => &mut self.flat[i].1,
            Err(_) => self.fresh.entry(page).or_insert(new),
        }
    }

    // pages new since the last read go into the sorted vec, in one pass
    fn merge(&mut self) {
        if self.fresh.is_empty() {
            return;
        }
        let fresh = std::mem::take(&mut self.fresh);
        let old = std::mem::take(&mut self.flat);
        let mut out = Vec::with_capacity(old.len() + fresh.len());
        let mut a = old.into_iter().peekable();
        let mut b = fresh.into_iter().peekable();
        loop {
            let take_a = match (a.peek(), b.peek()) {
                (Some(x), Some(y)) => x.0 < y.0,
                (Some(_), None) => true,
                (None, Some(_)) => false,
                (None, None) => break,
            };
            if let Some(e) = if take_a { a.next() } else { b.next() } {
                out.push(e);
            }
        }
        self.flat = out;
    }

    /// The pages in `[first, last)` with their askings, in page order.
    fn range(&mut self, first: usize, last: usize) -> Vec<(usize, V)> {
        if !self.is_flat() {
            return self.tree.range(first..last).map(|(&p, &w)| (p, w)).collect();
        }
        // a full merge only once enough new pages wait: each costs the whole vec
        if self.fresh.len() >= FRESH_MOST {
            self.merge();
        }
        let a = self.flat.partition_point(|e| e.0 < first);
        let b = self.flat.partition_point(|e| e.0 < last).max(a);
        let old = &self.flat[a..b];
        if last <= first || self.fresh.range(first..last).next().is_none() {
            return old.to_vec();
        }
        let mut out = Vec::with_capacity(old.len() + 16);
        let mut new = self.fresh.range(first..last).map(|(&p, &w)| (p, w)).peekable();
        for &e in old {
            while let Some(&n) = new.peek().filter(|n| n.0 < e.0) {
                out.push(n);
                new.next();
            }
            out.push(e);
        }
        out.extend(new);
        out
    }
}

/// Diagnostics: one in 2,048 of the askings the kernel's write watch
/// refused for memory the guards did not know (some 200 a frame in the
/// first rooms, each followed by a question to Windows about the memory):
/// what the memory is and what the guards hold of its first block.
#[cfg(feature = "diagnostics")]
fn failed_asking(start: usize, end: usize) {
    use windows_sys::Win32::System::Memory::{MEMORY_BASIC_INFORMATION, VirtualQuery};
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    if N.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % 2048 != 0 {
        return;
    }
    // SAFETY: plain data filled in by the query.
    let mut info: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: a valid out pointer of the right size.
    let n = unsafe { VirtualQuery(start as *const _, &mut info, size_of_val(&info)) };
    let (protected, clean, absent, viewed, aliased) = {
        let g = guards();
        let b = start / BLOCK;
        (g.protected.contains(&b), g.clean.contains(&b), g.absent.contains(&b), g.views.contains(&b), g.aliased.contains(&b))
    };
    tracing::info!(
        "a failed write-watch asking (one in 2048): {start:#x}+{:#x}; Windows says (answered {n}): base {:#x} region {:#x} state {:#x} type {:#x} protect {:#x} allocation protect {:#x}; \
         the guards of its first block: protected {protected} clean {clean} absent {absent} viewed {viewed} aliased {aliased}; is a view: {}",
        end - start,
        info.BaseAddress as usize,
        info.RegionSize,
        info.State,
        info.Type,
        info.Protect,
        info.AllocationProtect,
        is_view(start, end)
    );
}

impl Tracked {
    /// Takes changed pages of `[from, to)`, cut to the tracked range. A
    /// partial page stays changed so a later take can read its other bytes.
    pub(crate) fn take(&self, from: usize, to: usize) -> Vec<(usize, usize)> {
        let (from, to) = (from.max(self.start), to.min(self.end));
        if from >= to {
            return Vec::new();
        }
        let Some((_, pages)) = writes(from, to, true) else {
            return Vec::new();
        };
        let mut seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        let mut ranges: Vec<(usize, usize)> = Vec::new();
        for (page, write) in pages {
            if seen.get(&page).is_some_and(|&last| last >= write) {
                continue;
            }
            let (start, end) = ((page * PAGE).max(from), ((page + 1) * PAGE).min(to));
            if start >= end {
                continue;
            }
            // Keep partial pages dirty: the caller has not taken the rest.
            if (page * PAGE).max(self.start) >= from
                && ((page + 1) * PAGE).min(self.end) <= to
            {
                seen.insert(page, write);
            }
            match ranges.last_mut() {
                Some((_, last)) if *last == start => *last = end,
                _ => ranges.push((start, end)),
            }
        }
        ranges
    }
}

/// Guarded blocks of view memory.
const BLOCK: usize = PAGE;

/// Whether the block holding `start` is view memory the guards already
/// hold (read-only by our hand, or noted absent): asking the kernel's write
/// watch about it can only fail. `VICTORY_VIEW_ASK=1` (or
/// `VICTORY_GUARD_ASK=1`) asks the kernel anyway, as before.
fn known_view(start: usize) -> bool {
    static ASK: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let ask = *ASK.get_or_init(|| {
        ["VICTORY_VIEW_ASK", "VICTORY_GUARD_ASK"]
            .iter()
            .any(|n| std::env::var_os(n).is_some_and(|v| v == "1"))
    });
    if ask {
        return false;
    }
    let first = start / BLOCK;
    if view_memo() && VIEW_MEMO.with(|m| m.borrow_mut().holds(first)) {
        return true;
    }
    let g = guards();
    let view = g.protected.contains(&first) || ((seen_views() || view_known()) && g.views.contains(&first));
    if view && view_memo() {
        VIEW_MEMO.with(|m| m.borrow_mut().keep(first));
    }
    view || g.absent.contains(&first)
}

/// Whether [`known_view`] remembers, on each thread, the blocks it found to
/// be view memory until the runtime maps or unmaps title memory
/// (`VICTORY_VIEW_MEMO=1`; off by default).
///
/// Why: in the s2-prep-1 profile the submitting thread waited 6.4 ms a
/// frame for the guards' lock in [`known_view`] alone, asking about the
/// same blocks at every submission. A block stays view memory until
/// [`title_memory_remapped`]; blocks noted absent are not remembered (an
/// ordinary commit fills them with no word to us).
fn view_memo() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_VIEW_MEMO").is_some_and(|v| v == "1"))
}

/// Mappings and unmappings of title memory so far: [`ViewMemo`] forgets
/// what it held when this moves.
static REMAPPINGS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// A thread's view blocks, as of a count of [`REMAPPINGS`].
#[derive(Default)]
struct ViewMemo {
    remappings: u64,
    blocks: std::collections::HashSet<usize>,
}

impl ViewMemo {
    fn current(&mut self) {
        let now = REMAPPINGS.load(std::sync::atomic::Ordering::Acquire);
        if now != self.remappings {
            self.remappings = now;
            self.blocks.clear();
        }
    }

    fn holds(&mut self, block: usize) -> bool {
        self.current();
        self.blocks.contains(&block)
    }

    fn keep(&mut self, block: usize) {
        self.current();
        self.blocks.insert(block);
    }
}

thread_local! {
    static VIEW_MEMO: std::cell::RefCell<ViewMemo> = std::cell::RefCell::new(ViewMemo::default());
}

struct Guards {
    /// Blocks we made read-only.
    protected: BTreeSet<usize>,
    /// Of those, the ones not written since a copy took them.
    clean: BTreeSet<usize>,
    /// Blocks already copied as zeros. While they remain wholly uncommitted,
    /// the copy can skip hundreds of MB of failing kernel reads per world frame.
    absent: BTreeSet<usize>,
    /// Blocks whose physical page was guarded together with its other views
    /// (those views' blocks are in it too): a write to one opens them all.
    /// A write range can be huge (`fread`), so only these are looked up.
    aliased: BTreeSet<usize>,
    /// blocks guarded since their last remap: still views (VICTORY_GUARD_SEEN)
    views: BTreeSet<usize>,
    /// with VICTORY_FAULT_LOG: inside `covered`, the blocks not clean
    unclean: BTreeSet<usize>,
    covered: crate::guard_log::Covered,
    /// blocks written lately, by frame (VICTORY_HOT_COUNT, VICTORY_HOT_OPEN)
    faults: crate::guard_log::Faults,
}

impl Guards {
    fn set_clean(&mut self, block: usize) {
        self.clean.insert(block);
        self.unclean.remove(&block);
    }

    fn set_clean_range(&mut self, from: usize, to: usize) {
        self.clean.extend(from..to);
        if !self.unclean.is_empty() {
            let gone: Vec<usize> = self.unclean.range(from..to).copied().collect();
            for block in gone {
                self.unclean.remove(&block);
            }
        }
    }

    /// `block` is no longer clean (opened, or committed).
    fn set_unclean(&mut self, block: usize) {
        crate::push::changed(block);
        self.clean.remove(&block);
        if self.covered.holds(block, block + 1) {
            self.unclean.insert(block);
        }
    }

    /// Blocks `[from, to)` mapped or unmapped: no stretch holds them now.
    fn uncover(&mut self, from: usize, to: usize) {
        if !self.covered.touches(from, to) {
            return;
        }
        crate::guard_log::count(crate::guard_log::REMAP_CUTS, 1);
        self.covered.remove(from, to);
        let gone: Vec<usize> = self.unclean.range(from..to).copied().collect();
        for block in gone {
            self.unclean.remove(&block);
        }
    }
}

/// Whether a block once guarded stays known as view memory while its
/// guard is open (`VICTORY_VIEW_KNOWN=1`; off by default).
///
/// A range whose first block the title wrote since the last check has
/// that block neither read-only nor noted absent, so nothing says it is a
/// view: the kernel's write watch is asked and fails, and then Windows is
/// asked what the memory is (`is_view`), a walk of the page tables to the
/// end of the pages that share the first one's state. In runs 222 and 225
/// that was 338-450 failed askings a frame and 6.5-7.8 ms a frame of the
/// submitting thread in `VirtualQuery`. With the switch the block is
/// remembered from the time it was first made read-only until the runtime
/// maps or unmaps it ([`title_memory_remapped`]).
///
/// `VICTORY_VIEW_KNOWN=alt`: twenty seconds with, twenty without, in turn,
/// each change said in the log, for measuring it in one scene. (Blocks
/// noted in one phase are forgotten at a remapping in either.)
// the same set as VICTORY_GUARD_SEEN: either switch keeps it
fn view_known() -> bool {
    /// 0 off, 1 on, 2 in turn.
    static MODE: std::sync::OnceLock<u8> = std::sync::OnceLock::new();
    match *MODE.get_or_init(|| match std::env::var("VICTORY_VIEW_KNOWN").as_deref() {
        Ok("1") => 1,
        Ok("alt") => 2,
        _ => 0,
    }) {
        0 => false,
        1 => true,
        _ => {
            static EPOCH: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
            let on = (EPOCH.get_or_init(std::time::Instant::now).elapsed().as_secs() / 20) % 2 == 1;
            static LAST: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(2);
            if LAST.swap(u8::from(on), std::sync::atomic::Ordering::Relaxed) != u8::from(on) {
                tracing::info!(
                    "view memory stays known while its guard is open from now (VICTORY_VIEW_KNOWN=alt): {}",
                    if on { "ON" } else { "off, as usual" }
                );
            }
            on
        }
    }
}

static GUARDS: Mutex<Guards> = Mutex::new(Guards {
    protected: BTreeSet::new(),
    clean: BTreeSet::new(),
    absent: BTreeSet::new(),
    aliased: BTreeSet::new(),
    views: BTreeSet::new(),
    unclean: BTreeSet::new(),
    covered: crate::guard_log::Covered::new(),
    faults: crate::guard_log::Faults::new(),
});

fn guards() -> std::sync::MutexGuard<'static, Guards> {
    let mut g = GUARDS.lock().unwrap_or_else(|e| e.into_inner());
    // blocks the handler opened without the lock: opened here first (VICTORY_ONWRITE_LOCKFREE),
    // once every block taken is pushed
    crate::guard_free::wait_flights();
    crate::guard_free::drain(|block, frame| opened_free(&mut g, block, frame));
    g
}

// as the locked handler opens a block, for one the handler took (`crate::guard_free`)
fn opened_free(g: &mut Guards, block: usize, frame: u64) {
    if open(g, block * BLOCK, block * BLOCK + 1) && crate::guard_log::hot_count() {
        g.faults.note(block, frame);
    }
}

/// Whether the block `page` is read-only by our hand and unwritten since a
/// copy took it (diagnostics of `VICTORY_BACK_OWN_CHECK`).
pub(crate) fn clean(page: usize) -> bool {
    guards().clean.contains(&page)
}

/// For view memory: the pages of `[start, end)`'s blocks written since
/// they were last guarded (all of a block's pages), guarding them again
/// before the caller copies them. `None` if the memory is not a view.
fn guard(start: usize, end: usize) -> Option<Vec<usize>> {
    // A range whose first block the guards hold already (read-only by our
    // hand, or noted absent) is a view: no question to Windows. The
    // question walks the page tables to the end of the pages that share the
    // first one's state, and a guarded view is one long run of read-only
    // pages: 0.6 ms for the title's 174 MB heap, 16 times a frame, and most
    // of the 68 ms a frame of "guards" (run 58).
    // `VICTORY_GUARD_ASK=1` asks every time, as before.
    static ASK: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let known = !*ASK.get_or_init(|| std::env::var_os("VICTORY_GUARD_ASK").is_some_and(|v| v == "1")) && {
        let g = guards();
        let first = start / BLOCK;
        g.protected.contains(&first) || g.absent.contains(&first) || ((seen_views() || view_known()) && g.views.contains(&first))
    };
    if !known && !is_view(start, end) {
        return None;
    }
    install();
    let mut g = guards();
    if !crate::guard_log::fault_log() && !g.covered.is_empty() {
        // switched off (VICTORY_ALT): forget the stretches, faults are not kept for them now
        g.covered = crate::guard_log::Covered::new();
        g.unclean.clear();
    }
    let mut written = Vec::new();
    let mut run: Option<usize> = None;
    let (first, last) = (start / BLOCK, end.div_ceil(BLOCK));
    let frame = crate::frame::current();
    if crate::guard_log::commit_notice() && crate::guard_log::retry_frame(frame) {
        crate::guard_log::count(crate::guard_log::NET_PASSES, 1);
    }
    if crate::guard_log::hot_count() {
        g.faults.census(frame, BLOCK);
    }
    let log = crate::guard_log::fault_log();
    if log && g.covered.holds(first, last) {
        // the blocks not clean, as the handler and the passes left them
        let unclean: Vec<usize> = g.unclean.range(first..last).copied().collect();
        crate::guard_log::count(crate::guard_log::RANGES_COVERED, 1);
        crate::guard_log::count(crate::guard_log::FROM_LOG, unclean.len());
        let mut uncommitted_until = first;
        let mut prev = None;
        for block in unclean {
            if let (Some(from), Some(p)) = (run, prev)
                && block != p + 1
            {
                run = None;
                end_run(&mut g, from, p + 1);
            }
            prev = Some(block);
            visit(
                &mut g,
                block,
                frame,
                &mut run,
                &mut written,
                &mut uncommitted_until,
            );
        }
        if let (Some(from), Some(p)) = (run, prev) {
            end_run(&mut g, from, p + 1);
        }
        return Some(written);
    }
    if log {
        let why = if g.covered.touches(first, last) {
            crate::guard_log::RANGES_PART
        } else {
            crate::guard_log::RANGES_NEW
        };
        crate::guard_log::count(why, 1);
        crate::guard_log::count(crate::guard_log::WALKED, last - first);
    }
    // The range's clean blocks, in order: read off the set in one pass,
    // not looked up one block at a time. A frame asks about some 10 GB of
    // view memory, 150,000 blocks, nearly all clean: the lookups were most
    // of the 60-90 ms a frame of "guards" (run 54). What the loop adds to
    // the set meanwhile is the block it stands at or ones before it.
    // counted before it is listed: most ranges are clean through (VICTORY_GUARD_COUNT)
    if guard_count() && g.clean.range(first..last).count() == last - first {
        if crate::guard_log::fault_log() {
            g.covered.add(first, last);
        }
        return Some(written);
    }
    let clean: Vec<usize> = g.clean.range(first..last).copied().collect();
    if clean.len() == last - first {
        // Nothing written, mapped or new since the range was last taken.
        if crate::guard_log::fault_log() {
            g.covered.add(first, last);
        }
        return Some(written);
    }
    let mut uncommitted_until = first;
    let mut next_clean = 0;
    for block in first..last {
        while next_clean < clean.len() && clean[next_clean] < block {
            next_clean += 1;
        }
        if next_clean < clean.len() && clean[next_clean] == block {
            if let Some(from) = run.take() {
                end_run(&mut g, from, block);
            }
            continue;
        }
        visit(
            &mut g,
            block,
            frame,
            &mut run,
            &mut written,
            &mut uncommitted_until,
        );
    }
    if let Some(from) = run.take() {
        end_run(&mut g, from, last);
    }
    if crate::guard_log::fault_log() {
        // looked through: from now on the blocks not clean are kept
        let clean: BTreeSet<usize> = g.clean.range(first..last).copied().collect();
        for block in first..last {
            if !clean.contains(&block) {
                g.unclean.insert(block);
            }
        }
        g.covered.add(first, last);
    }
    Some(written)
}

/// [`guard`] at a block that is not clean: reported written unless it is
/// still uncommitted, then noted clean if still read-only, or added to the
/// open run to guard again (`run`), unless left open as hot.
fn visit(
    g: &mut Guards,
    block: usize,
    frame: u64,
    run: &mut Option<usize>,
    written: &mut Vec<usize>,
    uncommitted_until: &mut usize,
) {
    let mut open = true;
    if g.absent.contains(&block) {
        if crate::guard_log::commit_notice() && !crate::guard_log::retry_frame(frame) {
            // commits are told (title_memory_committed): still absent
            crate::guard_log::count(crate::guard_log::ASK_SKIPPED, 1);
            open = false;
        } else {
            let net = crate::guard_log::commit_notice();
            // Ordinary commits need no runtime mapping notification. Reuse
            // this asking's query for the following blocks in the same region.
            if block >= *uncommitted_until {
                *uncommitted_until = crate::wide_guard::uncommitted_end_asked(block)
                    .unwrap_or_else(|| uncommitted_end(block));
            }
            open = block >= *uncommitted_until;
            if net {
                crate::guard_log::count(crate::guard_log::NET_ASKED, 1);
                crate::guard_log::count(crate::guard_log::NET_FOUND, open as usize);
            }
            if open {
                g.absent.remove(&block);
            }
        }
    }
    if !open {
        if let Some(from) = run.take() {
            end_run(g, from, block);
        }
        return;
    }
    written.extend(block * BLOCK / PAGE..(block + 1) * BLOCK / PAGE);
    if g.protected.contains(&block) {
        // Written through the runtime, still read-only.
        g.set_clean(block);
        if let Some(from) = run.take() {
            end_run(g, from, block);
        }
        return;
    }
    if stays_open(g, block, frame, crate::guard_log::hot_open()) {
        // hot: left open, written again at the next pass
        if let Some(from) = run.take() {
            end_run(g, from, block);
        }
        return;
    }
    run.get_or_insert(block);
}

/// With VICTORY_HOT_OPEN: whether `block`, written in most frames lately
/// and shown at no other view, is left open.
fn stays_open(g: &mut Guards, block: usize, frame: u64, on: bool) -> bool {
    if !on
        || crate::guard_log::retry_frame(frame)
        || !g.faults.hot(block, frame)
        || g.aliased.contains(&block)
        || crate::inplace::pages_of(block * BLOCK).len() >= 2
    {
        return false;
    }
    // counted written this frame: it does not fault while open
    g.faults.note(block, frame);
    crate::guard_log::count(crate::guard_log::HOT, 1);
    true
}

/// Guards run `[from, to)` again (or notes it for after the flush).
fn end_run(g: &mut Guards, from: usize, to: usize) {
    if !crate::wide_guard::later(from, to) {
        protect(g, from, to);
    }
}

/// Whether [`guard`] counts a range's clean blocks before it lists them
/// (`VICTORY_GUARD_COUNT=1`; off by default): a range clean through, as
/// nearly all are, then costs no list (the count still walks the set). In
/// the s2-prep-1 profile those lists, walk and memory, were some 4 of the
/// 7 ms a frame of set and list work in `writes`. The answers are the same.
fn guard_count() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_GUARD_COUNT").is_some_and(|v| v == "1"))
}

/// skip is_view's query for blocks opened since we guarded them (VICTORY_GUARD_SEEN=1; off by default)
fn seen_views() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_GUARD_SEEN").is_some_and(|v| v == "1"))
}

/// Whether `[start, end)` is view memory. The title's views lie in
/// placeholders, mapped and unmapped block by block as it streams, so a
/// range can begin in a block that is not mapped right now: the first
/// pages of the range that are there decide. A range with nothing there
/// counts as a view (its blocks are noted absent, and looked at again when
/// the runtime maps them).
///
/// Looking at the first address only took such a range for "not a view":
/// its copy was marked not watched for good and copied whole at every
/// refresh, around its unmapped pages (run 54: 300 such ranges, 570 MB
/// copied a frame for them, 255 copies a frame reading around holes).
/// `VICTORY_GUARD_HOLES=0` looks at the first address only, as before.
fn is_view(start: usize, end: usize) -> bool {
    use windows_sys::Win32::System::Memory::{
        MEM_COMMIT, MEM_MAPPED, MEMORY_BASIC_INFORMATION, VirtualQuery,
    };
    static HOLES: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let holes = *HOLES.get_or_init(|| std::env::var_os("VICTORY_GUARD_HOLES").is_none_or(|v| v != "0"));
    let mut at = start;
    loop {
        // SAFETY: plain data filled in by the query.
        let mut info: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: a valid out pointer of the right size.
        let n = unsafe { VirtualQuery(at as *const _, &mut info, size_of_val(&info)) };
        if n == 0 {
            return false;
        }
        if info.Type == MEM_MAPPED {
            return true;
        }
        // Committed memory that is no view (the title's heaps, ours), or
        // the answer as it was before.
        if info.State == MEM_COMMIT || !holes {
            return false;
        }
        // A hole (a placeholder, or free): on to what follows it.
        let next = (info.BaseAddress as usize).saturating_add(info.RegionSize);
        if next <= at || next >= end {
            return true;
        }
        at = next;
    }
}

/// Makes blocks `[from, to)` read-only and notes them guarded and clean;
/// block by block if they span views, leaving out any that fail, and
/// noting those not committed (unmapped) absent. The same physical pages
/// at every other view of them are made read-only too: a write through
/// one of those is this memory written, and only a fault can say so.
/// `VICTORY_VIEW_GUARD=0` leaves the other views as they are, as before.
fn protect(g: &mut Guards, from: usize, to: usize) {
    protect_here(g, from, to);
    protect_aliases(g, from, to);
}

/// [`protect`]'s other views of blocks `[from, to)`.
fn protect_aliases(g: &mut Guards, from: usize, to: usize) {
    if !view_guard() {
        return;
    }
    for (base, bytes) in crate::inplace::views_between(from.saturating_mul(BLOCK), to.saturating_mul(BLOCK)) {
        let first = (base / BLOCK).max(from);
        let last = (base.saturating_add(bytes).div_ceil(BLOCK)).min(to);
        for block in first..last {
            let shown = crate::inplace::pages_of(block * BLOCK);
            // One view (nothing else to guard), or the page the title shows
            // wherever it has nothing (thousands of views): each view of
            // that one is guarded by itself.
            if shown.len() < 2 || shown.len() > MANY_VIEWS {
                continue;
            }
            #[cfg(feature = "diagnostics")]
            ALIASES[0].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            for alias in shown {
                let alias = alias / BLOCK;
                g.aliased.insert(alias);
                if alias != block {
                    protect_alias(g, alias);
                }
            }
        }
    }
}

/// Views of one page past which its views are not guarded together.
pub(crate) const MANY_VIEWS: usize = 64;

/// Whether all the views of a guarded page are guarded together
/// (`VICTORY_VIEW_GUARD`).
fn view_guard() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_VIEW_GUARD").is_none_or(|v| v != "0"))
}

/// Diagnostics: blocks guarded together with their other views, blocks
/// opened because another view of theirs was written, and blocks opened
/// because a view of theirs was mapped; since the start.
#[cfg(feature = "diagnostics")]
pub(crate) static ALIASES: [std::sync::atomic::AtomicU64; 3] = [const { std::sync::atomic::AtomicU64::new(0) }; 3];

/// Makes `block` read-only: another view of pages a copy has just taken
/// through its own view, so that a write through this one faults too. It is
/// not noted clean. The copies of title memory at this view have not taken
/// the pages: noted clean, the block would not be reported written at their
/// next check and they would keep the bytes from before the write (`guard`
/// reports a block that is read-only and not clean as written, then notes it
/// clean). A block that is clean stays so: had the pages been written since
/// its copies took them, `open` would have opened it.
fn protect_alias(g: &mut Guards, block: usize) {
    use windows_sys::Win32::System::Memory::{PAGE_READONLY, VirtualProtect};
    if g.protected.contains(&block) {
        return;
    }
    let mut old = 0;
    // SAFETY: title view memory; only its protection changes.
    if unsafe { VirtualProtect((block * BLOCK) as *const _, BLOCK, PAGE_READONLY, &mut old) } != 0 {
        g.protected.insert(block);
        crate::guard_free::mark(block);
        if seen_views() || view_known() {
            g.views.insert(block);
        }
    }
}

/// [`protect`] for one range, without the other views of its pages.
fn protect_here(g: &mut Guards, from: usize, to: usize) {
    let done = set_read_only(from, to);
    protect_noted(g, from, to, done);
}

/// Makes blocks `[from, to)` read-only; false if that failed.
pub(crate) fn set_read_only(from: usize, to: usize) -> bool {
    use windows_sys::Win32::System::Memory::{PAGE_READONLY, VirtualProtect};
    let mut old = 0;
    // SAFETY: title view memory; only its protection changes.
    unsafe {
        VirtualProtect(
            (from * BLOCK) as *const _,
            (to - from) * BLOCK,
            PAGE_READONLY,
            &mut old,
        ) != 0
    }
}

/// [`protect_here`] once [`set_read_only`] of the whole of `[from, to)`
/// said `done`.
fn protect_noted(g: &mut Guards, from: usize, to: usize, done: bool) {
    let note = |g: &mut Guards, from: usize, to: usize| {
        g.protected.extend(from..to);
        crate::guard_free::mark_range(from, to);
        g.set_clean_range(from, to);
        if seen_views() || view_known() {
            g.views.extend(from..to);
        }
    };
    if done {
        note(g, from, to);
        return;
    }
    for block in from..to {
        if set_read_only(block, block + 1) {
            note(g, block, block + 1);
        } else {
            #[cfg(feature = "diagnostics")]
            let error = std::io::Error::last_os_error();
            if block < uncommitted_end(block) {
                g.absent.insert(block);
            } else {
                // Diagnostics: a committed block that will not be made
                // read-only is reported written at every asking and never
                // known to the guards (one in 1,024 said).
                #[cfg(feature = "diagnostics")]
                {
                    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
                    if N.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % 1024 == 0 {
                        tracing::info!("a block of title memory could not be made read-only (one in 1024): {:#x}: {error}", block * BLOCK);
                    }
                }
            }
        }
    }
}

/// With [`crate::wide_guard`]: the guards' lock held while `set` makes
/// `runs` (blocks, in order, apart) read-only, then each noted guarded as
/// [`protect`] notes it, in order. `set` must not run other wide work.
pub(crate) fn protect_runs(
    runs: &[(usize, usize)],
    set: impl FnOnce(&[(usize, usize)]) -> Vec<bool>,
) {
    let mut g = guards();
    let done = set(runs);
    for (&(from, to), &done) in runs.iter().zip(&done) {
        protect_noted(&mut g, from, to, done);
        protect_aliases(&mut g, from, to);
    }
}

/// With [`crate::wide_guard`]: the blocks noted absent in `ranges`
/// (addresses), each range's in order.
pub(crate) fn absent_in(ranges: &[(usize, usize)]) -> Vec<Vec<usize>> {
    let g = guards();
    if g.absent.is_empty() {
        return Vec::new();
    }
    ranges
        .iter()
        .map(|&(start, end)| {
            g.absent
                .range(start / BLOCK..end.div_ceil(BLOCK))
                .copied()
                .collect()
        })
        .filter(|v: &Vec<usize>| !v.is_empty())
        .collect()
}

/// [`uncommitted_end`] for [`crate::wide_guard`].
pub(crate) fn uncommitted_end_of(block: usize) -> usize {
    uncommitted_end(block)
}

/// The end (exclusive) of the whole uncommitted blocks starting at `block`.
fn uncommitted_end(block: usize) -> usize {
    use windows_sys::Win32::System::Memory::{
        MEM_FREE, MEM_RESERVE, MEMORY_BASIC_INFORMATION, VirtualQuery,
    };
    // SAFETY: plain data filled in by the query.
    let mut info: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
    crate::guard_log::count(crate::guard_log::QUERIES, 1);
    // SAFETY: a valid out pointer of the right size.
    let n = unsafe { VirtualQuery((block * BLOCK) as *const _, &mut info, size_of_val(&info)) };
    if n != 0 && matches!(info.State, MEM_RESERVE | MEM_FREE) {
        (info.BaseAddress as usize).saturating_add(info.RegionSize) / BLOCK
    } else {
        block
    }
}

/// Gives write access back to the guarded blocks of `[start, end)` and
/// notes them written; true if any was guarded. The same physical pages at
/// every other view of them are opened too: the write that faulted (or is
/// ours, `crate::host::copy_mapped`) goes through one view, and every
/// guard over the pages must copy them again.
fn open(g: &mut Guards, start: usize, end: usize) -> bool {
    let any = open_here(g, start, end);
    if start >= end || g.aliased.is_empty() {
        return any;
    }
    let aliased: Vec<usize> = g.aliased.range(start / BLOCK..end.div_ceil(BLOCK)).copied().collect();
    for block in aliased {
        g.aliased.remove(&block);
        for alias in crate::inplace::pages_of(block * BLOCK) {
            if alias / BLOCK != block && open_here(g, alias, alias.saturating_add(BLOCK)) {
                #[cfg(feature = "diagnostics")]
                ALIASES[1].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            g.aliased.remove(&(alias / BLOCK));
        }
    }
    any
}

/// [`open`] for one range, without the other views of its pages.
fn open_here(g: &mut Guards, start: usize, end: usize) -> bool {
    use windows_sys::Win32::System::Memory::{PAGE_READWRITE, VirtualProtect};
    let blocks: Vec<usize> = g
        .protected
        .range(start / BLOCK..end.div_ceil(BLOCK))
        .copied()
        .collect();
    for &block in &blocks {
        g.protected.remove(&block);
        crate::guard_free::unmark(block);
        g.set_unclean(block);
        let mut old = 0;
        // SAFETY: a block we made read-only.
        unsafe { VirtualProtect((block * BLOCK) as *const _, BLOCK, PAGE_READWRITE, &mut old) };
    }
    !blocks.is_empty()
}

/// Before our own write into title memory `[at, at + len)` through the
/// kernel (which does not fault into our handler): opens its guarded blocks.
/// The runtime calls it (as `kernel_will_write`) before the title's kernel
/// reads into its own memory.
pub fn writing(at: usize, len: usize) {
    crate::burst::wrote(at, len);
    // The length is the title's (`fread`'s size times count, saturated).
    open(&mut guards(), at, at.saturating_add(len));
    // A placed texture there changes with no descriptor written.
    crate::descriptor::host::remapped(at, len);
}

/// What `VICTORY_PAGE_WRITES` asks to be done with a write the guards do
/// not see once its bytes are there ([`title_memory_written`]), as bits:
/// 1, the guarded blocks under it are opened and count as written (host
/// copies of buffers and textures read through tables take them again);
/// 2, the placed textures picked by index that lie under it are due again;
/// 4, both also for reads to plain addresses when they have finished.
/// Unset or `guards`: bit 1 alone. `0`: nothing, counted only (the default
/// until run 199). `1`: bits 1 and 2. `slots`: bit 2 alone. `all`:
/// everything.
/// (Bit 1 by default: in every run 10,000 to 50,000 guarded blocks of host
/// copies lay under such writes and kept the bytes from before; runs 197
/// and 198 opened them, 9,766 and 10,796 blocks, with nothing worse for
/// it. Bit 2 makes texture slots due and is the one that flooded the
/// queue in an earlier form: still asked for by name.)
pub(crate) fn page_writes() -> u8 {
    static MODE: std::sync::OnceLock<u8> = std::sync::OnceLock::new();
    *MODE.get_or_init(|| match std::env::var("VICTORY_PAGE_WRITES").as_deref().map(str::trim) {
        Ok("0") => 0,
        Ok("1") => 3,
        Ok("slots") => 2,
        Ok("all") => 7,
        _ => 1,
    })
}

/// Since the start: writes into physical pages through the runtime's own
/// views that landed at addresses the title has mapped (one for each run
/// of addresses); their bytes; guarded blocks that lay under them (host
/// copies that would otherwise keep the bytes from before); reads to
/// plain addresses that finished.
pub(crate) static WRITTEN: [std::sync::atomic::AtomicU64; 4] = [const { std::sync::atomic::AtomicU64::new(0) }; 4];

/// The bytes of a write the guards do not see are there: title memory
/// `[at, at + len)`. `pages`: DirectStorage wrote them into physical pages
/// through a view of the runtime's own, and this is an address where the
/// title has those pages mapped (no title address faulted, nothing was
/// mapped or unmapped; the shortcut this file's header names). Otherwise a
/// read to a plain address has finished, of which [`writing`] was told
/// before it began.
///
/// Upstream's c75fdba does the same for its trackers (its `Writing` marks
/// the pages once more when dropped; page destinations are reported at
/// each address showing them). Here it is counted always and acted on as
/// [`page_writes`] says. Called without the runtime's page-table lock;
/// takes the guards' lock, then, alone, the lock of the ranges kept for
/// the texture pass.
pub fn title_memory_written(at: usize, len: usize, pages: bool) {
    use std::sync::atomic::Ordering::Relaxed;
    if len == 0 {
        return;
    }
    crate::burst::wrote(at, len);
    let mode = page_writes();
    let end = at.saturating_add(len);
    if pages {
        WRITTEN[0].fetch_add(1, Relaxed);
        WRITTEN[1].fetch_add(len as u64, Relaxed);
    } else {
        WRITTEN[3].fetch_add(1, Relaxed);
        if mode & 4 == 0 {
            return;
        }
    }
    {
        let mut g = guards();
        let guarded = g.protected.range(at / BLOCK..end.div_ceil(BLOCK)).count() as u64;
        if pages {
            WRITTEN[2].fetch_add(guarded, Relaxed);
        }
        if mode & 1 != 0 && guarded != 0 {
            open(&mut g, at, end);
        }
    }
    // A placed texture there changed with no descriptor written.
    crate::descriptor::host::page_written(at, len);
    if pages {
        crate::texture_hold::landed(at as u64, len as u64);
    }
}

/// The runtime mapped a view of the physical bytes `[offset, offset + len)`
/// of `section` (`crate::inplace::title_memory_view`): the view is
/// writable, so the same bytes guarded at another view must be checked
/// again — and are, the pages noted written, which also has the next check
/// guard the new view (`protect`).
pub(crate) fn views_mapped(section: usize, offset: u64, len: usize) {
    if !view_guard() {
        return;
    }
    let ranges = crate::inplace::views_of_section(section, offset, len);
    if ranges.is_empty() {
        return;
    }
    let mut g = guards();
    for &(at, n) in &ranges {
        if open_here(&mut g, at, at.saturating_add(n)) {
            #[cfg(feature = "diagnostics")]
            ALIASES[2].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

/// Held shared by a read of title memory in place, and alone by the runtime
/// while it changes what title memory shows.
///
/// The runtime does not change a mapping in one step. To unmap one page of
/// a view it unmaps the whole view and maps what stays again
/// (`xmem.rs` `unmap`), and a view it maps is a reservation until the next
/// call commits it (`map`, then `commit`). For that moment pages the title
/// never gave up are "reserved, never committed", and a thread reading
/// them in place takes an access violation: run 71 in `Layout::detile`,
/// run 74 in the hash before it, both under `Staging::refresh_now`, which
/// had asked whether the texture was all mapped and been told yes. Asking
/// cannot help; not changing the mapping while such a read is under way
/// does.
static MAPPING: std::sync::RwLock<()> = std::sync::RwLock::new(());

/// What the runtime holds while it changes what title memory shows
/// ([`title_memory_remapping`]).
pub struct Remapping(#[allow(dead_code)] std::sync::RwLockWriteGuard<'static, ()>, std::time::Instant);

/// remappings since the frame line: count, us waiting for the lock, us held, longest hold, bytes remapped
pub(crate) static REMAPS: [std::sync::atomic::AtomicU64; 5] = [const { std::sync::atomic::AtomicU64::new(0) }; 5];

impl Drop for Remapping {
    fn drop(&mut self) {
        use std::sync::atomic::Ordering::Relaxed;
        let held = self.1.elapsed().as_micros() as u64;
        REMAPS[2].fetch_add(held, Relaxed);
        REMAPS[3].fetch_max(held, Relaxed);
        // the frame log's remap time (fxceless counts from the asking: here from the lock got)
        crate::framelog::add(crate::framelog::T::RemapUs, held);
    }
}

/// The runtime is about to map, unmap or move physical pages of title
/// memory: until what it returns is dropped, no read of title memory in
/// place is under way and none begins ([`title_memory_reading`]). Taken
/// around the whole change, the unmapping and the mapping again.
pub fn title_memory_remapping() -> Remapping {
    use std::sync::atomic::Ordering::Relaxed;
    let asked = std::time::Instant::now();
    let guard = MAPPING.write().unwrap_or_else(|e| e.into_inner());
    let got = std::time::Instant::now();
    REMAPS[0].fetch_add(1, Relaxed);
    REMAPS[1].fetch_add((got - asked).as_micros() as u64, Relaxed);
    crate::framelog::add(crate::framelog::T::Remaps, 1);
    crate::framelog::add(crate::framelog::T::RemapWaitUs, (got - asked).as_micros() as u64);
    Remapping(guard, got)
}

/// What a read of title memory in place holds ([`title_memory_reading`]).
pub struct Reading(#[allow(dead_code)] std::sync::RwLockReadGuard<'static, ()>);

/// For reading title memory in place: until what it returns is dropped,
/// the runtime changes no mapping, so memory found mapped stays mapped.
/// Not to be taken twice by one thread, nor held across anything that
/// maps title memory.
pub fn title_memory_reading() -> Reading {
    Reading(MAPPING.read().unwrap_or_else(|e| e.into_inner()))
}

/// [`title_memory_reading`] if it can be had at once: `None` while the
/// runtime changes a mapping (or waits to), for a caller that has another
/// way to read and would rather not wait.
pub fn title_memory_reading_now() -> Option<Reading> {
    match MAPPING.try_read() {
        Ok(guard) => Some(Reading(guard)),
        Err(std::sync::TryLockError::Poisoned(e)) => Some(Reading(e.into_inner())),
        Err(std::sync::TryLockError::WouldBlock) => None,
    }
}

/// The runtime mapped or unmapped title memory `[at, at + len)`: what it
/// shows is new and read-write, so it is no longer guarded (nor known
/// unmapped).
pub fn title_memory_remapped(at: usize, len: usize) {
    crate::burst::wrote(at, len);
    REMAPS[4].fetch_add(len as u64, std::sync::atomic::Ordering::Relaxed);
    // A placed texture there arrives, goes or changes with no descriptor
    // written.
    crate::descriptor::host::remapped(at, len);
    // The views the runtime told us of there are no longer what it shows.
    crate::inplace::views_gone(at, len);
    // And nothing stays in place over it.
    crate::inplace::title_memory_freeing(at, len);
    let mut g = guards();
    let range = at / BLOCK..at.saturating_add(len).div_ceil(BLOCK);
    crate::push::changed_pages(range.start, range.end);
    let absent: Vec<usize> = g.absent.range(range.clone()).copied().collect();
    for block in absent {
        g.absent.remove(&block);
    }
    g.uncover(range.start, range.end);
    let blocks: Vec<usize> = g.protected.range(range).copied().collect();
    for block in blocks {
        g.protected.remove(&block);
        crate::guard_free::unmark(block);
        g.clean.remove(&block);
    }
    let aliased: Vec<usize> = g.aliased.range(at / BLOCK..at.saturating_add(len).div_ceil(BLOCK)).copied().collect();
    for block in aliased {
        g.aliased.remove(&block);
    }
    if view_known() {
        // theirs: a remapped range is a view or a hole in one, so it stays known (VICTORY_VIEW_KNOWN)
        g.views.extend(at / BLOCK..at.saturating_add(len).div_ceil(BLOCK));
        REMAPPINGS.fetch_add(1, std::sync::atomic::Ordering::Release);
        return;
    }
    let views: Vec<usize> = g.views.range(at / BLOCK..at.saturating_add(len).div_ceil(BLOCK)).copied().collect();
    for block in views {
        g.views.remove(&block);
    }
    // under the guards' lock: a block remembered before this was found before it
    REMAPPINGS.fetch_add(1, std::sync::atomic::Ordering::Release);
}

/// The title committed memory `[at, at + len)` (XMem): blocks there noted
/// absent are so no longer, and the next pass takes them
/// (`VICTORY_COMMIT_NOTICE`, which stops asking Windows about absent
/// blocks at every pass).
pub fn title_memory_committed(at: usize, len: usize) {
    crate::burst::wrote(at, len);
    if len == 0 {
        return;
    }
    let mut g = guards();
    if g.absent.is_empty() {
        return;
    }
    let blocks: Vec<usize> = g
        .absent
        .range(at / BLOCK..at.saturating_add(len).div_ceil(BLOCK))
        .copied()
        .collect();
    crate::guard_log::count(crate::guard_log::COMMITS_NOTICED, blocks.len());
    for block in blocks {
        g.absent.remove(&block);
        g.set_unclean(block);
    }
}

/// With the frame line, when a guard-log switch is on: what the guard
/// passes did ([`crate::guard_log::guard_line`]).
pub(crate) fn guard_line(frames: f64) {
    if !crate::guard_log::any_on() {
        return;
    }
    let state = {
        let g = guards();
        (g.unclean.len(), g.covered.len(), g.absent.len())
    };
    crate::guard_log::guard_line(frames, BLOCK, state);
}

/// Registers the handler that takes writes to guarded blocks, once, ahead
/// of the crash reporter's.
fn install() {
    use windows_sys::Win32::Foundation::{EXCEPTION_ACCESS_VIOLATION, NTSTATUS};
    use windows_sys::Win32::System::Diagnostics::Debug::{
        AddVectoredExceptionHandler, EXCEPTION_CONTINUE_EXECUTION, EXCEPTION_CONTINUE_SEARCH,
        EXCEPTION_POINTERS,
    };
    unsafe extern "system" fn on_write(info: *mut EXCEPTION_POINTERS) -> i32 {
        // SAFETY: the system passes the exception being dispatched.
        let r = unsafe { &*(*info).ExceptionRecord };
        const WRITE: usize = 1;
        if r.ExceptionCode != EXCEPTION_ACCESS_VIOLATION as NTSTATUS
            || r.NumberParameters < 2
            || r.ExceptionInformation[0] != WRITE
        {
            return EXCEPTION_CONTINUE_SEARCH;
        }
        let at = r.ExceptionInformation[1];
        // marked ours and read-only: opened without the lock, noted at the next drain
        // in flight from before the take until the push: a pass waits for it (guards())
        let flight = crate::guard_free::Flight::enter();
        if crate::guard_free::take(at / BLOCK) {
            if open_taken(at / BLOCK, flight) {
                #[cfg(feature = "diagnostics")]
                crate::copystat::add(crate::copystat::C::GuardFaults, 1);
                crate::framelog::add(crate::framelog::T::GuardFaults, 1);
                return EXCEPTION_CONTINUE_EXECUTION;
            }
        } else {
            // out of flight before the locked way below waits for flights
            drop(flight);
        }
        // Held across both checks: another thread may have opened the block
        // since this one faulted, and a copy may have guarded it again since
        // (then it is ours again, not read-only for some other reason).
        let mut g = guards();
        if open(&mut g, at, at + 1) {
            if crate::guard_log::hot_count() {
                g.faults.note(at / BLOCK, crate::frame::current());
            }
            #[cfg(feature = "diagnostics")]
            crate::copystat::add(crate::copystat::C::GuardFaults, 1);
            crate::framelog::add(crate::framelog::T::GuardFaults, 1);
            return EXCEPTION_CONTINUE_EXECUTION;
        }
        if writable(at) {
            EXCEPTION_CONTINUE_EXECUTION
        } else {
            EXCEPTION_CONTINUE_SEARCH
        }
    }
    static ONCE: std::sync::Once = std::sync::Once::new();
    // SAFETY: the handler only touches our guard state and protections.
    ONCE.call_once(|| unsafe {
        crate::guard_free::init();
        AddVectoredExceptionHandler(1, Some(on_write));
    });
}

/// In the handler, for a block [`crate::guard_free::take`] gave it: read-write
/// now, and queued for the guards' next holder; under the lock if the queue
/// is full.
/// False if the block was not read-only after all (remapped since it was
/// marked): its protection is put back and the caller goes the locked way.
fn open_taken(block: usize, flight: Option<crate::guard_free::Flight>) -> bool {
    use windows_sys::Win32::System::Memory::{PAGE_READONLY, PAGE_READWRITE, VirtualProtect};
    let mut old = 0;
    // SAFETY: a block we made read-only.
    if unsafe { VirtualProtect((block * BLOCK) as *const _, BLOCK, PAGE_READWRITE, &mut old) } == 0 {
        return false;
    }
    if old != PAGE_READONLY {
        let mut was = 0;
        // SAFETY: as it was a moment ago.
        unsafe { VirtualProtect((block * BLOCK) as *const _, BLOCK, old, &mut was) };
        return false;
    }
    #[cfg(test)]
    crate::guard_free::park_here();
    let frame = crate::frame::current();
    if !crate::guard_free::push(block, frame) {
        // out of flight first: guards() waits for every flight
        drop(flight);
        opened_free(&mut guards(), block, frame);
    }
    true
}

/// Whether `at` is committed read-write memory now.
fn writable(at: usize) -> bool {
    use windows_sys::Win32::System::Memory::{
        MEM_COMMIT, MEMORY_BASIC_INFORMATION, PAGE_READWRITE, VirtualQuery,
    };
    // SAFETY: plain data filled in by the query.
    let mut info: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: a valid out pointer of the right size.
    let n = unsafe { VirtualQuery(at as *const _, &mut info, size_of_val(&info)) };
    n != 0 && info.State == MEM_COMMIT && info.Protect == PAGE_READWRITE
}

#[cfg(test)]
mod tests {
    use super::*;

    /// VICTORY_WIDE_ASKS: a run asked without the journal answers the
    /// written 4 KiB pages of write-watched memory, once (reset), and two
    /// reservations apart are asked by halves, each alone left out as
    /// `ask_run` leaves it.
    #[test]
    fn a_run_asked_alone_gives_the_written_pages_once() {
        use windows_sys::Win32::System::Memory::{
            MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE, VirtualAlloc, VirtualFree,
        };
        const MEM_WRITE_WATCH: u32 = 0x0020_0000;
        // SAFETY: fresh memory of our own, released below.
        let base = unsafe {
            VirtualAlloc(
                std::ptr::null(),
                8 * PAGE,
                MEM_RESERVE | MEM_COMMIT | MEM_WRITE_WATCH,
                PAGE_READWRITE,
            )
        } as usize;
        assert_ne!(base, 0);
        let first = base / PAGE;
        let _ = kernel_written(first, first + 8);
        // SAFETY: inside the allocation.
        unsafe {
            ((base + PAGE + 3 * KERNEL_PAGE) as *mut u8).write_volatile(1);
            ((base + 5 * PAGE) as *mut u8).write_volatile(1);
        }
        let mut out = Vec::new();
        run_alone(&[(first, first + 2), (first + 4, first + 8)], &mut out);
        assert_eq!(
            out,
            vec![(
                first,
                first + 8,
                vec![base + PAGE + 3 * KERNEL_PAGE, base + 5 * PAGE]
            )]
        );
        let mut again = Vec::new();
        run_alone(&[(first, first + 2), (first + 4, first + 8)], &mut again);
        assert_eq!(again, vec![(first, first + 8, Vec::new())]);
        // a run across our memory and memory that is not watched: by halves, none alone
        let mut split = Vec::new();
        run_alone(&[(first, first + 2), (first + 64, first + 65)], &mut split);
        assert!(split.is_empty());
        // SAFETY: our allocation.
        unsafe { VirtualFree(base as *mut _, 0, MEM_RELEASE) };
    }

    /// VICTORY_VIEW_MEMO: a remembered view block is forgotten once title
    /// memory is mapped or unmapped.
    #[test]
    fn a_remembered_view_is_forgotten_at_a_remapping() {
        let mut memo = ViewMemo::default();
        memo.keep(7);
        assert!(memo.holds(7));
        assert!(!memo.holds(8));
        REMAPPINGS.fetch_add(1, std::sync::atomic::Ordering::Release);
        assert!(!memo.holds(7));
        memo.keep(9);
        assert!(memo.holds(9));
    }
    use windows_sys::Win32::System::Memory::{
        MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE, VirtualAlloc, VirtualFree,
    };
    const MEM_WRITE_WATCH: u32 = 0x20_0000;

    #[test]
    fn lean_pre_askings_leave_out_turned_down_ranges() {
        let (down, kept) = (0x7ff0_0000_0000usize, 0x7ff1_0000_0000usize);
        JOURNAL.lock().unwrap().refused.insert(down / PAGE);
        let out = lean_ranges(&[(down, down + PAGE), (kept, kept + PAGE), (kept, kept)]);
        assert_eq!(out, [(kept, kept + PAGE)]);
        JOURNAL.lock().unwrap().refused.remove(&(down / PAGE));
    }

    #[test]
    fn piece_masks_and_needs() {
        assert_eq!(piece_mask(1, PAGE, 2 * PAGE), u16::MAX);
        assert_eq!(piece_mask(1, PAGE + 4096, PAGE + 4097), 0b10);
        assert_eq!(piece_mask(1, PAGE - 5, PAGE + 1), 0b1);
        assert_eq!(piece_mask(1, 0, PAGE), 0);
        assert_eq!(piece_mask(0, 15 * 4096 + 1, 99 * PAGE), 0x8000);
        let n = Pieces { last: 7, older: 5, mask: 0b100 };
        assert_eq!(pieces_needed(6, 7, Some(n)), 0b100, "copied after the older writes");
        assert_eq!(pieces_needed(5, 7, Some(n)), 0b100);
        assert_eq!(pieces_needed(4, 7, Some(n)), u16::MAX, "an older write it lacks");
        assert_eq!(pieces_needed(0, 7, Some(n)), u16::MAX, "never copied");
        assert_eq!(pieces_needed(6, 8, Some(n)), u16::MAX, "the journal moved on");
        assert_eq!(pieces_needed(6, 7, None), u16::MAX);
    }

    #[test]
    fn written_pages_are_reported_to_every_asker() {
        // SAFETY: a fresh write-watched allocation, freed at the end.
        let p = unsafe {
            VirtualAlloc(
                std::ptr::null(),
                4 * PAGE,
                MEM_RESERVE | MEM_COMMIT | MEM_WRITE_WATCH,
                PAGE_READWRITE,
            )
        } as usize;
        assert_ne!(p, 0);
        let (start, end) = (p, p + 4 * PAGE);
        let (_, before) = writes(start, end, false).expect("watched");
        assert!(before.is_empty());
        // SAFETY: committed pages of ours.
        unsafe { ((p + PAGE + 8) as *mut u8).write(1) };
        let (now, after) = writes(start, end, false).expect("watched");
        assert_eq!(after, [(p / PAGE + 1, now)]);
        // Another asker after the kernel's bit was cleared still sees it.
        let (later, again) = writes(p + PAGE, p + 2 * PAGE, false).expect("watched");
        assert!(later > now);
        assert_eq!(again, [(p / PAGE + 1, now)]);
        let mut plain = vec![0u8; PAGE];
        assert!(
            writes(
                plain.as_mut_ptr() as usize,
                plain.as_ptr() as usize + PAGE,
                true
            )
            .is_none()
        );
        // SAFETY: our allocation.
        unsafe { VirtualFree(p as *mut _, 0, MEM_RELEASE) };
    }

    /// VICTORY_JOURNAL_FLAT: the flat journal answers every range as the
    /// tree does, through new pages, rewrites and merges in any order.
    #[test]
    fn a_flat_journal_answers_as_the_tree_does() {
        let (mut tree, mut flat) = (Pages::<u64>::new(journal_flat), Pages::<u64>::new(journal_flat));
        tree.mode = 1;
        flat.mode = 2;
        let mut seed = 0x9e37_79b9u32;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed as usize
        };
        for now in 1..4000u64 {
            let page = next() % 3000;
            assert_eq!(tree.insert(page, now), flat.insert(page, now), "insert of page {page}");
            if now % 7 == 0 {
                let a = next() % 3100;
                let b = a + next() % 400;
                assert_eq!(tree.range(a, b), flat.range(a, b), "range {a}..{b}");
            }
        }
        assert_eq!(tree.range(0, usize::MAX), flat.range(0, usize::MAX));
        assert!(flat.range(5, 5).is_empty() && flat.range(9, 3).is_empty());
    }

    /// VICTORY_ALT: pages that follow their switch change form whenever it
    /// flips, between any two uses, and answer as the tree does throughout.
    #[test]
    fn a_journal_whose_switch_flips_answers_as_the_tree_does() {
        static FLAT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        fn flat_now() -> bool {
            FLAT.load(std::sync::atomic::Ordering::Relaxed)
        }
        let (mut tree, mut live) = (Pages::<u64>::new(journal_flat), Pages::<u64>::new(flat_now));
        tree.mode = 1;
        let mut seed = 0x2545_f491u32;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed as usize
        };
        for now in 1..6000u64 {
            if now % 97 == 0 {
                FLAT.fetch_xor(true, std::sync::atomic::Ordering::Relaxed);
            }
            let page = next() % 3000;
            if now % 3 == 0 {
                *tree.entry(page, 0) += now;
                *live.entry(page, 0) += now;
            } else {
                assert_eq!(tree.insert(page, now), live.insert(page, now), "insert of page {page}");
            }
            if now % 5 == 0 {
                let a = next() % 3100;
                let b = a + next() % 400;
                assert_eq!(tree.range(a, b), live.range(a, b), "range {a}..{b}");
            }
        }
        assert_eq!(tree.range(0, usize::MAX), live.range(0, usize::MAX));
    }

    /// VICTORY_PIECES_FLAT: piece notes kept flat take the same updates
    /// through `entry` and answer every range as the tree does.
    #[test]
    fn flat_piece_notes_answer_as_the_tree_does() {
        let (mut tree, mut flat) = (Pages::<Pieces>::new(pieces_flat), Pages::<Pieces>::new(pieces_flat));
        tree.mode = 1;
        flat.mode = 2;
        let mut seed = 0x2545_f491u32;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed as usize
        };
        let key = |v: Vec<(usize, Pieces)>| v.into_iter().map(|(p, x)| (p, x.last, x.older, x.mask)).collect::<Vec<_>>();
        for now in 1..5000u64 {
            let page = next() % 2500;
            let mask = (next() & 0xffff) as u16;
            for t in [&mut tree, &mut flat] {
                let e = t.entry(page, Pieces { last: now - 1, older: now - 1, mask: u16::MAX });
                if e.last == now {
                    e.mask |= mask;
                } else {
                    e.older = e.last;
                    e.last = now;
                    e.mask = mask;
                }
            }
            if now % 5 == 0 {
                let a = next() % 2600;
                let b = a + next() % 300;
                assert_eq!(key(tree.range(a, b)), key(flat.range(a, b)), "range {a}..{b}");
            }
        }
        assert_eq!(key(tree.range(0, usize::MAX)), key(flat.range(0, usize::MAX)));
    }

    /// offline numbers: a 40k-page journal, ranges of 2,700 pages (the heap)
    #[test]
    #[ignore]
    fn bench_journal_ranges() {
        for mode in [1u8, 2] {
            let mut j = Pages::<u64>::new(journal_flat);
            j.mode = mode;
            for page in 0..40_000usize {
                j.insert(page * 3, 1);
            }
            let t = std::time::Instant::now();
            let mut n = 0;
            for i in 0..2000usize {
                let a = (i * 37) % 110_000;
                n += j.range(a, a + 8100).len();
                j.insert(a, i as u64);
            }
            println!(
                "bench journal {}: {:.1} us a range of {} pages",
                if mode == 1 { "tree" } else { "flat" },
                t.elapsed().as_secs_f64() * 1e6 / 2000.0,
                n / 2000
            );
        }
    }

    /// A title write that lands while a submission is prepared (after its
    /// asking, as with an early present while the title builds the next
    /// frame) is not lost to the asking's reset: the submission's own take
    /// misses it, the next submission's asking finds it, once.
    #[test]
    fn a_write_landing_mid_submit_is_found_by_the_next_asking() {
        // SAFETY: a fresh write-watched allocation, freed at the end.
        let p = unsafe {
            VirtualAlloc(
                std::ptr::null(),
                4 * PAGE,
                MEM_RESERVE | MEM_COMMIT | MEM_WRITE_WATCH,
                PAGE_READWRITE,
            )
        } as usize;
        assert_ne!(p, 0);
        let (a, b) = (p + 8, p + 2 * PAGE + 8);
        let copy = track(p, 4 * PAGE, false).expect("watched");
        let ranges = [(p, p + PAGE), (p + 2 * PAGE, p + 3 * PAGE)];
        // SAFETY: committed pages of ours.
        unsafe { (a as *mut u8).write(1) };
        // submission N: one asking (kernel bits read and reset), then its refreshes
        ask_ahead(&ranges);
        // the title writes mid-submit, after the reset
        // SAFETY: committed pages of ours.
        unsafe {
            (a as *mut u8).write(2);
            (b as *mut u8).write(3);
        }
        assert_eq!(copy.take(p, p + 3 * PAGE), [(p, p + PAGE)], "submission N's asking found what was before it");
        ask_ahead_done();
        // submission N+1 asks again: both mid-submit writes are found
        ask_ahead(&ranges);
        assert_eq!(
            copy.take(p, p + 3 * PAGE),
            [(p, p + PAGE), (p + 2 * PAGE, p + 3 * PAGE)],
            "a write after submission N's asking was dropped"
        );
        ask_ahead_done();
        // and only once
        assert!(copy.take(p, p + 3 * PAGE).is_empty());
        // kept, not freed: the global journal remembers these pages, and a reused address would hand them to the next test
    }

    // a kernel write (ReadFile) into watched memory that is no view: the write watch reports the pages
    #[test]
    fn a_kernel_read_into_watched_memory_is_found_by_the_write_watch() {
        use std::io::{Read, Seek, Write};
        use windows_sys::Win32::System::Memory::GetWriteWatch;
        // SAFETY: a fresh write-watched allocation, kept
        let p = unsafe { VirtualAlloc(std::ptr::null(), 4 * PAGE, MEM_RESERVE | MEM_COMMIT | MEM_WRITE_WATCH, PAGE_READWRITE) } as usize;
        assert_ne!(p, 0);
        // written pages of the allocation, reset as the journal's askings reset them
        let written = || {
            let mut found = [0usize; 64];
            let (mut count, mut granularity) = (found.len(), 0u32);
            // SAFETY: room for `count` answers
            let failed = unsafe { GetWriteWatch(1, p as *const _, 4 * PAGE, found.as_mut_ptr().cast(), &mut count, &mut granularity) } != 0;
            assert!(!failed);
            found[..count].iter().map(|&at| (at - p) / PAGE).collect::<BTreeSet<usize>>()
        };
        assert!(written().is_empty());
        // touched first: a page never touched counts as written at the kernel's first touch, read or write
        for at in (0..4 * PAGE).step_by(KERNEL_PAGE) {
            // SAFETY: committed pages of ours
            unsafe { ((p + at) as *mut u8).write(1) };
        }
        assert_eq!(written().len(), 4);
        assert!(written().is_empty());
        let mut file = tempfile::tempfile().unwrap();
        file.write_all(&[0x5a; 3 * 4096]).unwrap();
        file.seek(std::io::SeekFrom::Start(0)).unwrap();
        // SAFETY: committed pages of ours; the kernel writes them
        let into = unsafe { std::slice::from_raw_parts_mut((p + 2 * PAGE + 100) as *mut u8, 3 * 4096) };
        file.read_exact(into).unwrap();
        assert_eq!(into[4096], 0x5a);
        assert_eq!(written(), BTreeSet::from([2]), "a kernel read into watched memory left no write-watch bit");
        assert!(written().is_empty());
    }

    /// A kernel read into a guarded view block fails; opened first, it
    /// lands and the block counts as written.
    #[test]
    fn kernel_reads_into_guarded_blocks_after_opening() {
        use std::io::{Read, Seek, Write};
        use windows_sys::Win32::System::Memory::{
            CreateFileMappingW, FILE_MAP_WRITE, MapViewOfFile, UnmapViewOfFile,
        };
        // SAFETY: a pagefile section of two blocks, mapped read-write.
        let view = unsafe {
            let section = CreateFileMappingW(
                -1isize as _,
                std::ptr::null(),
                PAGE_READWRITE,
                0,
                (2 * BLOCK) as u32,
                std::ptr::null(),
            );
            assert!(!section.is_null());
            MapViewOfFile(section, FILE_MAP_WRITE, 0, 0, 2 * BLOCK).Value as usize
        };
        assert_ne!(view, 0);
        assert!(guard(view, view + 2 * BLOCK).is_some()); // now read-only
        let mut file = tempfile::tempfile().unwrap();
        file.write_all(b"from the disk").unwrap();
        // `ReadFile` inside: the kernel writes the view.
        let read = |file: &mut std::fs::File| {
            file.rewind().unwrap();
            // SAFETY: 13 bytes of our view, nothing else refers to them.
            let into = unsafe { std::slice::from_raw_parts_mut((view + 100) as *mut u8, 13) };
            file.read(into).is_ok_and(|n| n == 13)
        };
        assert!(
            !read(&mut file),
            "the kernel does not fault into our handler"
        );
        writing(view + 100, 13);
        assert!(read(&mut file));
        // SAFETY: the bytes just read.
        let landed = unsafe { std::slice::from_raw_parts((view + 100) as *const u8, 13) };
        assert_eq!(landed, b"from the disk");
        assert_eq!(
            guard(view, view + 2 * BLOCK),
            Some((view / PAGE..view / PAGE + 1).collect())
        );
        // SAFETY: our view.
        unsafe {
            UnmapViewOfFile(
                windows_sys::Win32::System::Memory::MEMORY_MAPPED_VIEW_ADDRESS {
                    Value: view as *mut _,
                },
            )
        };
        title_memory_remapped(view, 2 * BLOCK);
    }

    #[test]
    fn writes_in_partly_committed_view_blocks_are_reported() {
        use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
        use windows_sys::Win32::System::Memory::{
            CreateFileMappingW, FILE_MAP_WRITE, MEMORY_MAPPED_VIEW_ADDRESS, MapViewOfFile,
            SEC_RESERVE, UnmapViewOfFile,
        };

        const SIZE: usize = 3 * BLOCK;
        struct View(MEMORY_MAPPED_VIEW_ADDRESS);
        impl Drop for View {
            fn drop(&mut self) {
                title_memory_remapped(self.0.Value as usize, SIZE);
                // SAFETY: our view is still mapped.
                unsafe { UnmapViewOfFile(self.0) };
            }
        }
        // SAFETY: a new reserved pagefile section. The view holds its own
        // reference after we close the handle, including on assertion failure.
        let view = unsafe {
            let section = CreateFileMappingW(
                INVALID_HANDLE_VALUE,
                std::ptr::null(),
                PAGE_READWRITE | SEC_RESERVE,
                0,
                SIZE as u32,
                std::ptr::null(),
            );
            assert!(!section.is_null());
            let view = MapViewOfFile(section, FILE_MAP_WRITE, 0, 0, SIZE);
            CloseHandle(section);
            assert!(!view.Value.is_null());
            View(view)
        };
        let start = view.0.Value as usize;
        let commit = |at: usize, len| {
            // SAFETY: these pages belong to our reserved view.
            let committed = unsafe { VirtualAlloc(at as _, len, MEM_COMMIT, PAGE_READWRITE) };
            assert_eq!(committed as usize, at);
        };
        let early = start + BLOCK + 3 * KERNEL_PAGE;
        let late = start + 2 * BLOCK + 3 * KERNEL_PAGE;
        commit(start, BLOCK);
        commit(early, KERNEL_PAGE);
        let refresh = || guard(start, start + SIZE).unwrap();
        assert_eq!(refresh(), [start / PAGE, early / PAGE, late / PAGE]);
        // The last block is still entirely reserved.
        assert!(!refresh().contains(&(late / PAGE)));

        commit(late, KERNEL_PAGE); // Ordinary commit, without a runtime mapping notification.
        for at in [early, late] {
            assert!(writable(at));
            // SAFETY: each address is inside its committed read-write page.
            unsafe { (at as *mut u8).write_volatile(1) };
        }
        assert_eq!(refresh(), [early / PAGE, late / PAGE]);
    }

    /// A range of view memory whose first block is not mapped is a view
    /// all the same: its blocks are reported once, then the ones written
    /// or mapped since.
    #[test]
    fn a_range_that_begins_in_an_unmapped_block_is_guarded() {
        use windows_sys::Win32::System::Memory::{
            CreateFileMappingW, MEM_PRESERVE_PLACEHOLDER, MEM_REPLACE_PLACEHOLDER,
            MEM_RESERVE_PLACEHOLDER, MapViewOfFile3, PAGE_NOACCESS, UnmapViewOfFile2,
            VirtualAlloc2,
        };
        use windows_sys::Win32::System::Threading::GetCurrentProcess;
        // SAFETY: a fresh placeholder of three blocks and a pagefile
        // section of three; the middle block is mapped first.
        let (p, section) = unsafe {
            let p = VirtualAlloc2(
                GetCurrentProcess(),
                std::ptr::null(),
                3 * BLOCK,
                MEM_RESERVE | MEM_RESERVE_PLACEHOLDER,
                PAGE_NOACCESS,
                std::ptr::null_mut(),
                0,
            ) as usize;
            assert_ne!(p, 0);
            let section = CreateFileMappingW(
                -1isize as _,
                std::ptr::null(),
                PAGE_READWRITE,
                0,
                (3 * BLOCK) as u32,
                std::ptr::null(),
            );
            assert!(!section.is_null());
            (p, section)
        };
        let map = |block: usize| {
            // SAFETY: splits the placeholder at the block and maps the
            // section's block there.
            let view = unsafe {
                VirtualFree(
                    (p + block * BLOCK) as *mut _,
                    BLOCK,
                    MEM_RELEASE | MEM_PRESERVE_PLACEHOLDER,
                );
                MapViewOfFile3(
                    section,
                    GetCurrentProcess(),
                    (p + block * BLOCK) as *const _,
                    (block * BLOCK) as u64,
                    BLOCK,
                    MEM_REPLACE_PLACEHOLDER,
                    PAGE_READWRITE,
                    std::ptr::null_mut(),
                    0,
                )
            };
            assert_eq!(view.Value as usize, p + block * BLOCK);
            view
        };
        let pages = |block: usize| (p + block * BLOCK) / PAGE..(p + (block + 1) * BLOCK) / PAGE;
        let all: Vec<usize> = (0..3).flat_map(pages).collect();
        // Nothing mapped at all: a view with every block absent.
        assert!(is_view(p, p + 3 * BLOCK));
        let middle = map(1);
        title_memory_remapped(p + BLOCK, BLOCK);
        // The range begins in the placeholder, its middle block is a view.
        assert!(is_view(p, p + 3 * BLOCK));
        assert_eq!(guard(p, p + 3 * BLOCK), Some(all));
        assert_eq!(guard(p, p + 3 * BLOCK), Some(Vec::new()), "nothing written since");
        // SAFETY: inside the mapped middle block.
        unsafe { ((p + BLOCK + 8) as *mut u8).write_volatile(1) };
        assert_eq!(guard(p, p + 3 * BLOCK), Some(pages(1).collect()));
        let first = map(0);
        title_memory_remapped(p, BLOCK);
        assert_eq!(guard(p, p + 3 * BLOCK), Some(pages(0).collect()));
        assert_eq!(guard(p, p + 3 * BLOCK), Some(Vec::new()));
        // Memory of ours that is no view stays none.
        let mut local = [0u8; 16];
        let at = local.as_mut_ptr() as usize;
        assert!(!is_view(at, at + local.len()));
        // SAFETY: our views and placeholder.
        unsafe {
            UnmapViewOfFile2(GetCurrentProcess(), first, MEM_PRESERVE_PLACEHOLDER);
            UnmapViewOfFile2(GetCurrentProcess(), middle, MEM_PRESERVE_PLACEHOLDER);
            for block in 0..3 {
                VirtualFree((p + block * BLOCK) as *mut _, 0, MEM_RELEASE);
            }
            windows_sys::Win32::Foundation::CloseHandle(section);
        }
        title_memory_remapped(p, 3 * BLOCK);
    }

    /// Blocks of a placeholder between views are reported once, then not
    /// again (they cannot be guarded) until the runtime maps them.
    #[test]
    fn unmapped_blocks_are_reported_once_until_mapped() {
        use windows_sys::Win32::System::Memory::{
            CreateFileMappingW, MEM_PRESERVE_PLACEHOLDER, MEM_REPLACE_PLACEHOLDER,
            MEM_RESERVE_PLACEHOLDER, MapViewOfFile3, PAGE_NOACCESS, UnmapViewOfFile2,
            VirtualAlloc2,
        };
        use windows_sys::Win32::System::Threading::GetCurrentProcess;
        // SAFETY: a fresh placeholder of three blocks and a pagefile
        // section of two; block 0 shows the section's first block, as the
        // runtime maps physical pages.
        let (p, section) = unsafe {
            let p = VirtualAlloc2(
                GetCurrentProcess(),
                std::ptr::null(),
                3 * BLOCK,
                MEM_RESERVE | MEM_RESERVE_PLACEHOLDER,
                PAGE_NOACCESS,
                std::ptr::null_mut(),
                0,
            ) as usize;
            assert_ne!(p, 0);
            let section = CreateFileMappingW(
                -1isize as _,
                std::ptr::null(),
                PAGE_READWRITE,
                0,
                (2 * BLOCK) as u32,
                std::ptr::null(),
            );
            assert!(!section.is_null());
            (p, section)
        };
        let map = |block: usize| {
            // SAFETY: splits the placeholder at the block and maps the
            // section's block there.
            let view = unsafe {
                VirtualFree(
                    (p + block * BLOCK) as *mut _,
                    BLOCK,
                    MEM_RELEASE | MEM_PRESERVE_PLACEHOLDER,
                );
                MapViewOfFile3(
                    section,
                    GetCurrentProcess(),
                    (p + block * BLOCK) as *const _,
                    (block * BLOCK) as u64,
                    BLOCK,
                    MEM_REPLACE_PLACEHOLDER,
                    PAGE_READWRITE,
                    std::ptr::null_mut(),
                    0,
                )
            };
            assert_eq!(view.Value as usize, p + block * BLOCK);
            view
        };
        let first = map(0);
        let pages = |block: usize| (p + block * BLOCK) / PAGE..(p + (block + 1) * BLOCK) / PAGE;
        let all: Vec<usize> = (0..3).flat_map(pages).collect();
        assert_eq!(guard(p, p + 3 * BLOCK), Some(all));
        assert_eq!(
            guard(p, p + 3 * BLOCK),
            Some(Vec::new()),
            "nothing written since"
        );
        let second = map(1);
        title_memory_remapped(p + BLOCK, BLOCK);
        assert_eq!(guard(p, p + 3 * BLOCK), Some(pages(1).collect()));
        assert_eq!(guard(p, p + 3 * BLOCK), Some(Vec::new()));
        // SAFETY: our views and placeholder.
        unsafe {
            UnmapViewOfFile2(GetCurrentProcess(), first, MEM_PRESERVE_PLACEHOLDER);
            UnmapViewOfFile2(GetCurrentProcess(), second, MEM_PRESERVE_PLACEHOLDER);
            for block in 0..3 {
                VirtualFree((p + block * BLOCK) as *mut _, 0, MEM_RELEASE);
            }
            windows_sys::Win32::Foundation::CloseHandle(section);
        }
        title_memory_remapped(p, 3 * BLOCK);
    }

    /// A pagefile section mapped twice: two views of the same physical
    /// pages, the second left for the test; the section handle, the two
    /// addresses.
    fn two_views(bytes: usize) -> (usize, usize, usize) {
        use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
        use windows_sys::Win32::System::Memory::{
            CreateFileMappingW, FILE_MAP_WRITE, MapViewOfFile,
        };
        // SAFETY: a fresh pagefile section, mapped twice; the views keep
        // the section alive and the handle is closed by the caller.
        unsafe {
            let section = CreateFileMappingW(
                INVALID_HANDLE_VALUE,
                std::ptr::null(),
                PAGE_READWRITE,
                0,
                bytes as u32,
                std::ptr::null(),
            );
            assert!(!section.is_null());
            let one = MapViewOfFile(section, FILE_MAP_WRITE, 0, 0, bytes).Value as usize;
            let two = MapViewOfFile(section, FILE_MAP_WRITE, 0, 0, bytes).Value as usize;
            assert_ne!(one, 0);
            assert_ne!(two, 0);
            crate::inplace::title_memory_view(one, bytes, section as usize, 0);
            crate::inplace::title_memory_view(two, bytes, section as usize, 0);
            (section as usize, one, two)
        }
    }

    /// The section of [`two_views`] is closed and its views unmapped; the
    /// guard is told they are gone first.
    fn drop_views(section: usize, views: &[usize], bytes: usize) {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Memory::{MEMORY_MAPPED_VIEW_ADDRESS, UnmapViewOfFile};
        for &view in views {
            title_memory_remapped(view, bytes);
            // SAFETY: a view of the section mapped by `two_views`.
            unsafe {
                UnmapViewOfFile(MEMORY_MAPPED_VIEW_ADDRESS {
                    Value: view as *mut _,
                })
            };
        }
        // SAFETY: the section's last reference.
        unsafe { CloseHandle(section as *mut std::ffi::c_void) };
    }

    /// The title wrote the pages of a guarded texture through another view
    /// of them: the fault at that view is the guard's to see, and the
    /// guarded view's next check reports the pages written.
    #[test]
    fn a_write_through_another_view_is_reported_at_the_guarded_one() {
        const SIZE: usize = 2 * BLOCK;
        let (section, guarded, other) = two_views(SIZE);
        let pages = |block: usize| (guarded + block * BLOCK) / PAGE;
        // The first check takes it in (and every view of its pages).
        assert_eq!(guard(guarded, guarded + SIZE), Some(vec![pages(0), pages(1)]));
        assert_eq!(guard(guarded, guarded + SIZE), Some(Vec::new()), "nothing written yet");
        // Writing through the other view faults into the handler, which
        // opens that view and the guarded one: the next check sees it.
        // SAFETY: the second view, read-write once the handler has opened it.
        unsafe { ((other + BLOCK + 8) as *mut u8).write(1) };
        assert_eq!(guard(guarded, guarded + SIZE), Some(vec![pages(1)]));
        assert_eq!(guard(guarded, guarded + SIZE), Some(Vec::new()));
        drop_views(section, &[guarded, other], SIZE);
    }

    /// A view of guarded pages is mapped anew: it is writable, so the
    /// guarded view checks its pages again (and guards the new view).
    #[test]
    fn a_view_mapped_beside_a_guarded_one_makes_it_check_again() {
        const SIZE: usize = 2 * BLOCK;
        let (section, guarded, other) = two_views(SIZE);
        let pages = |block: usize| (guarded + block * BLOCK) / PAGE;
        assert_eq!(guard(guarded, guarded + SIZE), Some(vec![pages(0), pages(1)]));
        assert_eq!(guard(guarded, guarded + SIZE), Some(Vec::new()), "nothing written yet");
        title_memory_remapped(other, SIZE);
        crate::inplace::title_memory_view(other, SIZE, section, 0);
        assert_eq!(guard(guarded, guarded + SIZE), Some(vec![pages(0), pages(1)]));
        assert_eq!(guard(guarded, guarded + SIZE), Some(Vec::new()));
        drop_views(section, &[guarded, other], SIZE);
    }

    /// A write of the runtime's own, told at one view of the pages
    /// (`writing`): the other views' next checks report the pages written.
    #[test]
    fn a_write_told_at_one_view_is_reported_at_the_others() {
        const SIZE: usize = 2 * BLOCK;
        let (section, guarded, other) = two_views(SIZE);
        let pages = |block: usize| (guarded + block * BLOCK) / PAGE;
        assert_eq!(guard(guarded, guarded + SIZE), Some(vec![pages(0), pages(1)]));
        assert_eq!(guard(guarded, guarded + SIZE), Some(Vec::new()), "nothing written yet");
        writing(other + BLOCK, BLOCK);
        assert_eq!(guard(guarded, guarded + SIZE), Some(vec![pages(1)]));
        assert_eq!(guard(guarded, guarded + SIZE), Some(Vec::new()));
        // Half a page and the next: both blocks, not just the first's.
        writing(other + BLOCK / 2, BLOCK);
        assert_eq!(guard(guarded, guarded + SIZE), Some(vec![pages(0), pages(1)]));
        drop_views(section, &[guarded, other], SIZE);
    }

    /// Both views of the pages are checked, each for a copy of its own
    /// (the title shows one physical page at several addresses, and each
    /// address has its host copy): a write through either is reported at
    /// both, whichever is checked first. Guarding the pages for the first
    /// view's check must not note the second view clean, or its copy would
    /// keep the bytes from before the write.
    #[test]
    fn a_write_is_reported_at_every_checked_view() {
        const SIZE: usize = 2 * BLOCK;
        let (section, one, two) = two_views(SIZE);
        let pages = |view: usize, block: usize| (view + block * BLOCK) / PAGE;
        assert_eq!(guard(one, one + SIZE), Some(vec![pages(one, 0), pages(one, 1)]));
        assert_eq!(guard(two, two + SIZE), Some(vec![pages(two, 0), pages(two, 1)]), "the second view's first check");
        assert_eq!(guard(one, one + SIZE), Some(Vec::new()), "nothing written yet");
        assert_eq!(guard(two, two + SIZE), Some(Vec::new()), "nothing written yet");
        // SAFETY: the second view, read-write once the handler has opened it.
        unsafe { ((two + BLOCK + 8) as *mut u8).write(1) };
        assert_eq!(guard(one, one + SIZE), Some(vec![pages(one, 1)]));
        assert_eq!(guard(two, two + SIZE), Some(vec![pages(two, 1)]), "written through this view, checked second");
        // SAFETY: the first view, as above.
        unsafe { ((one + 8) as *mut u8).write(1) };
        assert_eq!(guard(two, two + SIZE), Some(vec![pages(two, 0)]));
        assert_eq!(guard(one, one + SIZE), Some(vec![pages(one, 0)]), "written through this view, checked second");
        assert_eq!(guard(one, one + SIZE), Some(Vec::new()));
        assert_eq!(guard(two, two + SIZE), Some(Vec::new()));
        drop_views(section, &[one, two], SIZE);
    }

    /// A page shown in more than `MANY_VIEWS` views is guarded at each by
    /// itself: the others stay writable, and nothing looks through them all.
    #[test]
    fn a_page_of_many_views_is_guarded_one_view_at_a_time() {
        let (section, one, two) = two_views(BLOCK);
        let filler = one + (1 << 45);
        for n in 0..MANY_VIEWS {
            crate::inplace::title_memory_view(filler + n * BLOCK, BLOCK, section, 0);
        }
        assert_eq!(guard(one, one + BLOCK), Some(vec![one / PAGE]));
        assert!(writable(two), "another view of it is left alone");
        // SAFETY: the guarded view, opened by the handler.
        unsafe { ((one + 8) as *mut u8).write(1) };
        assert_eq!(guard(one, one + BLOCK), Some(vec![one / PAGE]));
        title_memory_remapped(filler, MANY_VIEWS * BLOCK);
        drop_views(section, &[one, two], BLOCK);
    }

    fn no_guards() -> Guards {
        Guards {
            protected: BTreeSet::new(),
            clean: BTreeSet::new(),
            absent: BTreeSet::new(),
            aliased: BTreeSet::new(),
            views: BTreeSet::new(),
            unclean: BTreeSet::new(),
            covered: crate::guard_log::Covered::new(),
            faults: crate::guard_log::Faults::new(),
        }
    }

    /// VICTORY_FAULT_LOG: inside a stretch looked through, a block that
    /// leaves the clean set is kept as not clean, and one that joins it is
    /// not; outside, nothing is kept; a remapping forgets the stretch.
    #[test]
    fn the_blocks_not_clean_follow_the_clean_set_inside_covered_stretches() {
        let mut g = no_guards();
        g.set_unclean(5);
        assert!(g.unclean.is_empty(), "not covered: left to a full pass");
        g.covered.add(10, 20);
        g.set_clean_range(10, 20);
        g.set_unclean(12);
        g.set_unclean(13);
        assert_eq!(g.unclean.iter().copied().collect::<Vec<_>>(), [12, 13]);
        assert!(!g.clean.contains(&12));
        g.set_clean(12);
        assert_eq!(g.unclean.iter().copied().collect::<Vec<_>>(), [13]);
        g.set_unclean(14);
        g.set_clean_range(13, 15);
        assert!(g.unclean.is_empty());
        g.set_unclean(16);
        g.uncover(15, 17);
        assert!(g.unclean.is_empty() && !g.covered.holds(10, 20) && g.covered.holds(10, 15));
    }

    /// VICTORY_HOT_OPEN: a block written in 3 of the last 4 frames is left
    /// open by the pass and reported written at the next, except at a retry
    /// frame and for a block shown at other views.
    #[test]
    fn a_hot_block_is_left_open_and_reported_written() {
        let mut g = no_guards();
        // far from any test's memory
        let block = (0x7e00_0000_0000usize) / BLOCK;
        for frame in [101, 102, 103] {
            g.faults.note(block, frame);
        }
        let pass = |g: &mut Guards, frame: u64, on: bool| stays_open(g, block, frame, on);
        assert!(!pass(&mut g, 103, false), "off: guarded again");
        assert!(pass(&mut g, 103, true));
        // left open, it counts as written each frame and stays hot
        assert!(pass(&mut g, 104, true));
        assert!(pass(&mut g, 105, true));
        assert!(!pass(&mut g, 128, true), "a retry frame closes it");
        for frame in [127, 128, 129] {
            g.faults.note(block, frame);
        }
        g.aliased.insert(block);
        assert!(!pass(&mut g, 130, true), "other views: never left open");
        g.aliased.remove(&block);
        assert!(pass(&mut g, 130, true));
    }

    /// VICTORY_FAULT_LOG under load: threads write a guarded view and the
    /// other view of its pages while a checker takes what each check
    /// reports into a copy; after the writers stop and one last check, the
    /// copy is the memory.
    #[test]
    fn a_copy_kept_from_the_checks_matches_memory_under_writer_threads() {
        use std::sync::atomic::{AtomicBool, Ordering};
        const BLOCKS: usize = 8;
        const SIZE: usize = BLOCKS * BLOCK;
        let (section, one, two) = two_views(SIZE);
        let mut copy = vec![0u8; SIZE];
        let take = |copy: &mut Vec<u8>| {
            let pages = guard(one, one + SIZE).expect("a view");
            for page in pages {
                let at = page * PAGE - one;
                // SAFETY: inside the first view.
                let bytes = unsafe { std::slice::from_raw_parts((one + at) as *const u8, PAGE) };
                copy[at..at + PAGE].copy_from_slice(bytes);
            }
        };
        take(&mut copy);
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        let writers: Vec<_> = (0..4)
            .map(|t| {
                let stop = stop.clone();
                std::thread::spawn(move || {
                    let mut x = 0x9e37_79b9_7f4a_7c15u64 ^ t;
                    let mut n = 0u64;
                    while !stop.load(Ordering::Relaxed) || n < 2_000 {
                        x ^= x << 13;
                        x ^= x >> 7;
                        x ^= x << 17;
                        let view = if x & 1 == 0 { one } else { two };
                        let at = (x as usize >> 8) % SIZE;
                        // SAFETY: inside a view; the handler opens a guarded block.
                        unsafe { ((view + at) as *mut u8).write_volatile((x >> 40) as u8) };
                        n += 1;
                        if n > 200_000 {
                            break;
                        }
                    }
                })
            })
            .collect();
        for _ in 0..200 {
            take(&mut copy);
        }
        stop.store(true, Ordering::Relaxed);
        for w in writers {
            w.join().unwrap();
        }
        take(&mut copy);
        // SAFETY: the whole first view.
        let memory = unsafe { std::slice::from_raw_parts(one as *const u8, SIZE) };
        assert!(copy == memory, "a write was missed");
        drop_views(section, &[one, two], SIZE);
    }

    // VICTORY_ONWRITE_LOCKFREE: eight writers over two views of guarded blocks, faulting the
    // lock-free way or the locked way; every write is in the copy taken from the checks
    fn writers_against_checks(locked: bool) -> u64 {
        use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
        const BLOCKS: usize = 16;
        const SIZE: usize = BLOCKS * BLOCK;
        let (section, one, two) = two_views(SIZE);
        let mut copy = vec![0u8; SIZE];
        let take = |copy: &mut Vec<u8>| {
            let pages = guard(one, one + SIZE).expect("a view");
            for page in pages {
                let at = page * PAGE - one;
                // SAFETY: inside the first view.
                let bytes = unsafe { std::slice::from_raw_parts((one + at) as *const u8, PAGE) };
                copy[at..at + PAGE].copy_from_slice(bytes);
            }
        };
        take(&mut copy);
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        let writes = std::sync::Arc::new(AtomicU64::new(0));
        let writers: Vec<_> = (0..8)
            .map(|t| {
                let (stop, writes) = (stop.clone(), writes.clone());
                std::thread::spawn(move || {
                    crate::guard_free::LOCKED.with(|l| l.set(locked));
                    let mut x = 0x2545_f491_4f6c_dd1du64 ^ (t << 8) ^ u64::from(locked);
                    let mut n = 0u64;
                    while (!stop.load(Ordering::Relaxed) || n < 2_000) && n < 300_000 {
                        x ^= x << 13;
                        x ^= x >> 7;
                        x ^= x << 17;
                        let view = if x & 1 == 0 { one } else { two };
                        let at = (x as usize >> 8) % SIZE;
                        // SAFETY: inside a view; the handler opens a guarded block.
                        unsafe { ((view + at) as *mut u8).write_volatile((x >> 40) as u8) };
                        n += 1;
                    }
                    writes.fetch_add(n, Ordering::Relaxed);
                })
            })
            .collect();
        for _ in 0..300 {
            take(&mut copy);
        }
        stop.store(true, Ordering::Relaxed);
        for w in writers {
            w.join().unwrap();
        }
        take(&mut copy);
        // SAFETY: the whole first view.
        let memory = unsafe { std::slice::from_raw_parts(one as *const u8, SIZE) };
        assert!(copy == memory, "a write was missed (locked: {locked})");
        drop_views(section, &[one, two], SIZE);
        writes.load(Ordering::Relaxed)
    }

    // a handler stopped between its take and its push while a pass runs: the pass waits for it,
    // and a write landed meanwhile by another thread (no fault: the block is open) is reported
    #[test]
    fn a_pass_waits_for_a_block_taken_and_not_yet_queued() {
        use std::sync::atomic::Ordering;
        const SIZE: usize = 2 * BLOCK;
        let (section, one, two) = two_views(SIZE);
        assert!(guard(one, one + SIZE).is_some());
        assert!(guard(one, one + SIZE).is_some(), "taken: clean from here");
        crate::guard_free::GO.store(false, Ordering::SeqCst);
        crate::guard_free::PARKED.store(false, Ordering::SeqCst);
        let parked = std::thread::spawn(move || {
            crate::guard_free::PARK.with(|p| p.set(true));
            // SAFETY: inside the first view; faults and parks after the take
            unsafe { ((one + BLOCK + 8) as *mut u8).write_volatile(1) };
        });
        while !crate::guard_free::PARKED.load(Ordering::SeqCst) {
            std::thread::yield_now();
        }
        // open now: no fault, nothing queued
        // SAFETY: inside the first view.
        unsafe { ((one + BLOCK + 100) as *mut u8).write_volatile(2) };
        let pass = std::thread::spawn(move || guard(one, one + SIZE));
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert!(!pass.is_finished(), "the pass waits for the flight");
        crate::guard_free::GO.store(true, Ordering::SeqCst);
        parked.join().unwrap();
        let written = pass.join().unwrap().expect("a view");
        assert!(written.contains(&((one + BLOCK) / PAGE)), "the block taken and its later write are reported");
        drop_views(section, &[one, two], SIZE);
    }

    #[test]
    fn lock_free_opens_miss_no_write_under_eight_writers() {
        let taken = crate::guard_free::TAKEN[0].load(std::sync::atomic::Ordering::Relaxed);
        let t = std::time::Instant::now();
        let n = writers_against_checks(false);
        let free = t.elapsed();
        assert!(crate::guard_free::TAKEN[0].load(std::sync::atomic::Ordering::Relaxed) > taken, "the lock-free way was taken");
        let t = std::time::Instant::now();
        let m = writers_against_checks(true);
        let locked = t.elapsed();
        println!("eight writers against 301 checks: lock-free {n} writes in {free:?}, locked {m} writes in {locked:?}");
    }

    /// A write or mapping that runs past the end of the address space (a
    /// title's `fread` with a huge count) covers up to the end; the sum used
    /// to overflow. Up there no test has guarded blocks.
    #[test]
    fn ranges_past_the_address_space_end_do_not_overflow() {
        let top = usize::MAX - 16;
        writing(top, PAGE);
        title_memory_remapped(top, usize::MAX);
    }
}

#[cfg(test)]
mod mapping_tests {
    use super::{title_memory_reading, title_memory_remapping};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, mpsc};
    use std::time::Duration;

    #[test]
    fn a_change_of_mapping_counts_its_hold() {
        use std::sync::atomic::Ordering::Relaxed;
        let before = [0, 2, 3].map(|i| super::REMAPS[i].load(Relaxed));
        {
            let _changing = title_memory_remapping();
            std::thread::sleep(Duration::from_millis(20));
        }
        let after = [0, 2, 3].map(|i| super::REMAPS[i].load(Relaxed));
        assert!(after[0] > before[0], "the change was not counted");
        assert!(after[1] - before[1] >= 20_000, "the hold was not counted");
        assert!(after[2] >= 20_000, "the longest hold was not kept");
    }

    /// A change of mapping waits for a read in place that is under way, and
    /// a read in place waits for a change of mapping.
    #[test]
    fn reads_in_place_and_remappings_wait_for_each_other() {
        // A reader holds; the change does not begin until it lets go.
        let changed = Arc::new(AtomicBool::new(false));
        let reading = title_memory_reading();
        let (began, wait) = mpsc::channel();
        let change = {
            let changed = changed.clone();
            std::thread::spawn(move || {
                began.send(()).unwrap();
                let _changing = title_memory_remapping();
                changed.store(true, Ordering::SeqCst);
            })
        };
        wait.recv().unwrap();
        std::thread::sleep(Duration::from_millis(100));
        assert!(!changed.load(Ordering::SeqCst), "the mapping changed under a read in place");
        drop(reading);
        change.join().unwrap();
        assert!(changed.load(Ordering::SeqCst));
        // A change holds; the read does not begin until it is done.
        let read = Arc::new(AtomicBool::new(false));
        let changing = title_memory_remapping();
        let (began, wait) = mpsc::channel();
        let reader = {
            let read = read.clone();
            std::thread::spawn(move || {
                began.send(()).unwrap();
                let _reading = title_memory_reading();
                read.store(true, Ordering::SeqCst);
            })
        };
        wait.recv().unwrap();
        std::thread::sleep(Duration::from_millis(100));
        assert!(!read.load(Ordering::SeqCst), "a read in place began during a change of mapping");
        drop(changing);
        reader.join().unwrap();
        assert!(read.load(Ordering::SeqCst));
    }
}
