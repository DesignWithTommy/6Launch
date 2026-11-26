//! `VICTORY_MERGE_WIDE=1` (off by default): [`Shadow::merge_under_writes`]
//! compares a word at a time and reads and compares its pages on the kept
//! wide threads; what it puts where, and when it says settle, are the same.

use std::sync::atomic::Ordering;
use std::sync::Mutex;

use super::{MERGED, PAGE, Shadow, changed_runs, joined_over, merge_gap};

/// `VICTORY_MERGE_WIDE=1`.
pub(super) fn wide() -> bool {
    // live: read once per merge, whose result is the same either way
    static S: crate::switch::Switch = crate::switch::Switch::noted(
        "MERGE_WIDE",
        "title bytes under flights' shader writes are compared a word at a time on the wide threads (VICTORY_MERGE_WIDE=1)",
    );
    S.on()
}

/// `VICTORY_MERGE_DENSE=1` (off by default): the plain merge
/// ([`Shadow::merge_dense`]) compares a page a word at a time straight into
/// masked dwords or runs, with no byte runs in between and its counts
/// added once a page.
///
/// Why: in fast-15s the "asking once" stage was 37-47 ms a frame, most of
/// it this merge over 241-286 pages with 356,000-512,000 changed runs of
/// about 6 bytes (some 1,400 a page): a byte-by-byte run list, its dword
/// masks byte by byte, and per run a copy, a lock of the dirty list and
/// two counters.
pub(super) fn dense() -> bool {
    #[cfg(test)]
    if DENSE.with(std::cell::Cell::get) {
        return true;
    }
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        let on = std::env::var_os("VICTORY_MERGE_DENSE").is_some_and(|v| v == "1");
        if on {
            tracing::info!("title pages under flights' shader writes go straight from the word compare to masks or runs (VICTORY_MERGE_DENSE=1)");
        }
        on
    })
}

#[cfg(test)]
thread_local! {
    // a test's plain merges take the dense way
    static DENSE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

// pages held under one lock of the reference
const BATCH: usize = 64;
// pages to a piece of work
const JOB: usize = 4;

const LOW: u64 = 0x7f7f_7f7f_7f7f_7f7f;

#[inline]
fn load(s: &[u8], i: usize) -> u64 {
    u64::from_le_bytes(s[i..i + 8].try_into().expect("eight bytes"))
}

// 0x80 in each byte of x that is not zero
#[inline]
fn high_bits(x: u64) -> u64 {
    (((x & LOW) + LOW) | x) & !LOW
}

// bit k set where byte k is not zero
#[inline]
fn gather(t: u64) -> u8 {
    ((t >> 7).wrapping_mul(0x0102_0408_1020_4080) >> 56) as u8
}

/// [`changed_runs`], eight bytes at a time.
pub(super) fn runs(now: &[u8], was: &[u8]) -> Vec<(usize, usize)> {
    let n = now.len().min(was.len());
    let mut out = Vec::new();
    let (mut open, mut first) = (false, 0usize);
    let mut i = 0;
    while i + 8 <= n {
        if !open {
            while i + 32 <= n
                && (load(now, i) ^ load(was, i))
                    | (load(now, i + 8) ^ load(was, i + 8))
                    | (load(now, i + 16) ^ load(was, i + 16))
                    | (load(now, i + 24) ^ load(was, i + 24))
                    == 0
            {
                i += 32;
            }
            if i + 8 > n {
                break;
            }
        }
        let d = gather(high_bits(load(now, i) ^ load(was, i)));
        let before = (d << 1) | u8::from(open);
        // bytes where a run starts or ends, in order
        let mut edges = d ^ before;
        while edges != 0 {
            let k = edges.trailing_zeros() as usize;
            if d >> k & 1 != 0 {
                first = i + k;
            } else {
                out.push((first, i + k - first));
            }
            edges &= edges - 1;
        }
        open = d & 0x80 != 0;
        i += 8;
    }
    while i < n {
        let differs = now[i] != was[i];
        if differs && !open {
            first = i;
        } else if !differs && open {
            out.push((first, i - first));
        }
        open = differs;
        i += 1;
    }
    if open {
        out.push((first, n - first));
    }
    out
}

/// `crate::card_mask::dword_masks(at, &changed_runs(now, was))` onto `out`,
/// a dword at a time; the runs' count and bytes.
pub(super) fn masks(at: usize, now: &[u8], was: &[u8], out: &mut Vec<(u32, u32)>) -> (usize, usize) {
    let n = now.len().min(was.len());
    let mine = out.len();
    let (mut count, mut bytes, mut open) = (0usize, 0usize, false);
    let push = |out: &mut Vec<(u32, u32)>, word: usize, bits: u32| {
        let len = out.len();
        match out.last_mut() {
            Some(last) if len > mine && last.0 == word as u32 => last.1 |= bits,
            _ => out.push((word as u32, bits)),
        }
    };
    let mut byte = |out: &mut Vec<(u32, u32)>, i: usize, open: &mut bool| {
        let differs = now[i] != was[i];
        if differs {
            count += usize::from(!*open);
            bytes += 1;
            let b = at + i;
            push(out, b & !3, 0xffu32 << ((b & 3) * 8));
        }
        *open = differs;
    };
    let mut i = 0;
    while i < n && (at + i) % 4 != 0 {
        byte(out, i, &mut open);
        i += 1;
    }
    let (mut wide_count, mut wide_bytes) = (0usize, 0usize);
    while i + 8 <= n {
        let x = load(now, i) ^ load(was, i);
        if x == 0 {
            open = false;
            i += 8;
            continue;
        }
        let t = high_bits(x);
        let d = gather(t);
        let before = (d << 1) | u8::from(open);
        wide_count += (d & !before).count_ones() as usize;
        wide_bytes += d.count_ones() as usize;
        open = d & 0x80 != 0;
        let bm = (t >> 7) * 0xff;
        let (lo, hi) = (bm as u32, (bm >> 32) as u32);
        if lo != 0 {
            push(out, at + i, lo);
        }
        if hi != 0 {
            push(out, at + i + 4, hi);
        }
        i += 8;
    }
    while i < n {
        byte(out, i, &mut open);
        i += 1;
    }
    (count + wide_count, bytes + wide_bytes)
}

// f(0..n) on the wide threads, `JOB` at a time; back when all are done
fn each(n: usize, f: &(dyn Fn(usize) + Sync)) {
    if n <= JOB {
        (0..n).for_each(f);
        return;
    }
    // SAFETY: `super::wide` returns only once every job has run, so `f`
    // outlives them.
    let f: &'static (dyn Fn(usize) + Sync) = unsafe { std::mem::transmute(f) };
    let jobs = (0..n)
        .step_by(JOB)
        .map(|first| Box::new(move || (first..(first + JOB).min(n)).for_each(f)) as super::Job)
        .collect();
    super::wide(jobs);
}

// `Shadow::card_changed` of each run at copy offset `at`, under one lock
fn dirty_runs(dirty: &mut Vec<(usize, usize)>, at: usize, runs: &[(usize, usize)]) {
    for &(r, m) in runs {
        let (from, to) = (at + r, at + r + m);
        if from >= to {
            continue;
        }
        match dirty.last_mut() {
            Some(last) if from <= last.1 && to >= last.0 => *last = (last.0.min(from), last.1.max(to)),
            _ => dirty.push((from, to)),
        }
    }
}

#[derive(Default)]
struct Scan {
    ok: bool,
    runs: Vec<(usize, usize)>,
    entries: Vec<(u32, u32)>,
    count: usize,
    bytes: usize,
}

static SCRATCH: Mutex<Vec<u8>> = Mutex::new(Vec::new());

impl Shadow {
    /// With [`dense`], [`Shadow::merge_stale`]'s work for the page read
    /// into `title` at copy offset `at`, under the reference's lock
    /// (`before`): the same bytes go to the same places, and the same
    /// entries, runs and counts are noted. Whether the flights must
    /// settle.
    pub(super) fn merge_dense(&self, at: usize, title: &[u8], before: &mut Option<Vec<u8>>, runs_left: &mut usize, masked: bool) -> bool {
        let n = title.len();
        let counted = |count: usize, bytes: usize| {
            MERGED[1].fetch_add(count as u64, Ordering::Relaxed);
            MERGED[2].fetch_add(bytes as u64, Ordering::Relaxed);
        };
        if let Some(c) = &self.card {
            // SAFETY: inside the mapping of the copy's CPU side, which only the submitting thread writes.
            let held = unsafe { std::slice::from_raw_parts_mut(self.data.add(at), n) };
            if masked {
                let mut entries = Vec::new();
                let (count, bytes) = masks(at, title, held, &mut entries);
                let mut notes = c.masked.lock().unwrap_or_else(|e| e.into_inner());
                if notes.pending.len() + entries.len() <= crate::card_mask::MOST {
                    notes.pending.extend_from_slice(&entries);
                    drop(notes);
                    // the bytes outside the masks are equal already
                    held.copy_from_slice(title);
                    counted(count, bytes);
                    return false;
                }
            }
            let runs = joined_over(runs(title, held), merge_gap());
            if runs.len() > *runs_left {
                return true;
            }
            *runs_left -= runs.len();
            held.copy_from_slice(title);
            dirty_runs(&mut c.dirty.lock().unwrap_or_else(|e| e.into_inner()), at, &runs);
            counted(runs.len(), runs.iter().map(|r| r.1).sum());
            return false;
        }
        let Some(snapshot) = before.as_mut() else {
            return true;
        };
        // SAFETY: the snapshot's allocation holds the copy's bytes at the copy's offsets.
        let held = unsafe { std::slice::from_raw_parts_mut(snapshot.as_mut_ptr().add(at), n) };
        let runs = runs(title, held);
        for &(r, m) in &runs {
            // SAFETY: inside the copy's mapping; only the changed bytes, as the card may be writing beside them.
            unsafe { std::ptr::copy_nonoverlapping(title.as_ptr().add(r), self.data.add(at + r), m) };
        }
        held.copy_from_slice(title);
        counted(runs.len(), runs.iter().map(|r| r.1).sum());
        false
    }

    /// [`Shadow::merge_stale`] with the switch: pages are read and compared
    /// `BATCH` at a time under one lock of the reference, on the wide
    /// threads, then taken in order as the plain merge takes them.
    pub(super) fn merge_stale_wide(&self, a: usize, e: usize, stale: &[usize], now: u64, runs_left: &mut usize, masked: bool) -> bool {
        let (start, end) = (self.start as usize, (self.start + self.len) as usize);
        let base = start / PAGE;
        let pages: Vec<(usize, usize, usize)> = stale
            .iter()
            .filter_map(|&page| {
                let (lo, hi) = ((page * PAGE).max(a), ((page + 1) * PAGE).min(e));
                (lo < hi).then_some((page, lo, hi - lo))
            })
            .collect();
        if pages.is_empty() {
            return false;
        }
        let mut buf = std::mem::take(&mut *SCRATCH.lock().unwrap_or_else(|e| e.into_inner()));
        if buf.len() < BATCH * PAGE {
            buf = vec![0u8; BATCH * PAGE];
        }
        let settle = self.merge_batches(&pages, &mut buf, a, e, now, runs_left, masked, start, end, base);
        *SCRATCH.lock().unwrap_or_else(|e| e.into_inner()) = buf;
        settle
    }

    #[allow(clippy::too_many_arguments)]
    fn merge_batches(
        &self,
        pages: &[(usize, usize, usize)],
        buf: &mut [u8],
        a: usize,
        e: usize,
        now: u64,
        runs_left: &mut usize,
        masked: bool,
        start: usize,
        end: usize,
        base: usize,
    ) -> bool {
        let card = self.card.is_some();
        let buf_at = buf.as_mut_ptr() as usize;
        for batch in pages.chunks(BATCH) {
            let mut taken: Vec<usize> = Vec::new();
            let settle = {
                let mut before = self.before.lock().unwrap_or_else(|e| e.into_inner());
                let held_at = if card {
                    self.data as usize
                } else {
                    match before.as_mut() {
                        Some(snapshot) => snapshot.as_mut_ptr() as usize,
                        None => {
                            // as the plain merge: the first page that reads says settle
                            let mut title = vec![0u8; PAGE];
                            for &(_, lo, n) in batch {
                                if crate::host::read_into(lo as *const u8, &mut title[..n]) {
                                    MERGED[0].fetch_add(1, Ordering::Relaxed);
                                    return true;
                                }
                            }
                            continue;
                        }
                    }
                };
                let scans: Vec<Mutex<Scan>> = batch.iter().map(|_| Mutex::default()).collect();
                let gap = merge_gap();
                each(batch.len(), &|k| {
                    let (_, lo, n) = batch[k];
                    // SAFETY: page k's own part of the scratch.
                    let title = unsafe { std::slice::from_raw_parts_mut((buf_at + k * PAGE) as *mut u8, n) };
                    let mut scan = Scan::default();
                    if crate::host::read_into(lo as *const u8, title) {
                        scan.ok = true;
                        let at = lo - start;
                        // SAFETY: the reference's bytes at the copy's offsets; its lock is held
                        // and nothing writes it meanwhile.
                        let held = unsafe { std::slice::from_raw_parts((held_at + at) as *const u8, n) };
                        if card && masked {
                            let (count, bytes) = masks(at, title, held, &mut scan.entries);
                            (scan.count, scan.bytes) = (count, bytes);
                        } else {
                            scan.runs = if card { joined_over(runs(title, held), gap) } else { runs(title, held) };
                            scan.count = scan.runs.len();
                            scan.bytes = scan.runs.iter().map(|r| r.1).sum();
                        }
                    }
                    *scans[k].lock().unwrap_or_else(|e| e.into_inner()) = scan;
                });
                let mut scans: Vec<Scan> = scans.into_iter().map(|s| s.into_inner().unwrap_or_else(|e| e.into_inner())).collect();
                // pages taken (index into the batch) and whether their runs also go into the copy
                let mut apply: Vec<(usize, bool)> = Vec::new();
                let mut settle = false;
                for (k, &(page, lo, n)) in batch.iter().enumerate() {
                    if !scans[k].ok {
                        continue;
                    }
                    MERGED[0].fetch_add(1, Ordering::Relaxed);
                    let at = lo - start;
                    if let Some(c) = &self.card {
                        if masked {
                            let mut notes = c.masked.lock().unwrap_or_else(|e| e.into_inner());
                            if notes.pending.len() + scans[k].entries.len() <= crate::card_mask::MOST {
                                notes.pending.extend_from_slice(&scans[k].entries);
                                MERGED[1].fetch_add(scans[k].count as u64, Ordering::Relaxed);
                                MERGED[2].fetch_add(scans[k].bytes as u64, Ordering::Relaxed);
                                apply.push((k, false));
                                taken.push(page);
                                continue;
                            }
                            drop(notes);
                            // past the cap: the runs, as the plain merge
                            // SAFETY: as in the scan.
                            let (title, held) = unsafe {
                                (
                                    std::slice::from_raw_parts((buf_at + k * PAGE) as *const u8, n),
                                    std::slice::from_raw_parts((held_at + at) as *const u8, n),
                                )
                            };
                            scans[k].runs = joined_over(changed_runs(title, held), merge_gap());
                        }
                        let runs = &scans[k].runs;
                        if runs.len() > *runs_left {
                            settle = true;
                            break;
                        }
                        *runs_left -= runs.len();
                        dirty_runs(&mut c.dirty.lock().unwrap_or_else(|e| e.into_inner()), at, runs);
                        MERGED[1].fetch_add(runs.len() as u64, Ordering::Relaxed);
                        MERGED[2].fetch_add(runs.iter().map(|r| r.1 as u64).sum::<u64>(), Ordering::Relaxed);
                        apply.push((k, false));
                    } else {
                        MERGED[1].fetch_add(scans[k].count as u64, Ordering::Relaxed);
                        MERGED[2].fetch_add(scans[k].bytes as u64, Ordering::Relaxed);
                        apply.push((k, true));
                    }
                    taken.push(page);
                }
                let data_at = self.data as usize;
                let scans = &scans;
                each(apply.len(), &|j| {
                    let (k, into_copy) = apply[j];
                    let (_, lo, n) = batch[k];
                    let at = lo - start;
                    let title = (buf_at + k * PAGE) as *const u8;
                    if into_copy {
                        for &(r, m) in &scans[k].runs {
                            // SAFETY: inside the copy's mapping; only the changed bytes, as the
                            // card may be writing beside them.
                            unsafe { std::ptr::copy_nonoverlapping(title.add(r), (data_at + at + r) as *mut u8, m) };
                        }
                    }
                    // the reference takes the title's bytes; the ones not in a run are equal already
                    // SAFETY: the reference's bytes of this page, under its lock.
                    unsafe { std::ptr::copy_nonoverlapping(title, (held_at + at) as *mut u8, n) };
                });
                drop(before);
                settle
            };
            for page in taken {
                self.took_page(page, base, start, end, a, e, now);
            }
            if settle {
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Card, Shadow, changed_runs};
    use super::{DENSE, PAGE, masks, runs};
    use std::sync::Mutex;
    use windows::Win32::Graphics::Direct3D::D3D_FEATURE_LEVEL_11_0;
    use windows::Win32::Graphics::Direct3D12::*;
    use windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC;
    use windows::Win32::Graphics::Dxgi::*;
    use windows_sys::Win32::System::Memory as mem;

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> usize {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0 as usize
        }
    }

    // a title page rewritten the way the game does: short runs, long equal stretches, whole words
    fn scribble(rng: &mut Rng, bytes: &mut [u8], density: usize) {
        let n = bytes.len();
        if n == 0 {
            return;
        }
        for _ in 0..density {
            let at = rng.next() % n;
            let len = match rng.next() % 4 {
                0 => 1,
                1 => 1 + rng.next() % 4,
                2 => 1 + rng.next() % 40,
                _ => 8 * (1 + rng.next() % 6),
            };
            for b in &mut bytes[at..(at + len).min(n)] {
                *b ^= 1 + (rng.next() % 255) as u8;
            }
        }
    }

    #[test]
    fn word_runs_are_the_byte_runs() {
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        for case in 0..3000 {
            let n = match case % 5 {
                0 => rng.next() % 40,
                1 => rng.next() % 300,
                2 => PAGE,
                _ => rng.next() % 4096,
            };
            let was: Vec<u8> = (0..n).map(|_| rng.next() as u8).collect();
            let mut now = was.clone();
            let density = 1 + rng.next() % 64;
            scribble(&mut rng, &mut now, density);
            if case % 7 == 0 {
                now.iter_mut().for_each(|b| *b = !*b);
            }
            assert_eq!(runs(&now, &was), changed_runs(&now, &was), "case {case}");
        }
    }

    #[test]
    fn word_masks_are_the_masks_of_the_byte_runs() {
        let mut rng = Rng(0x2545_f491_4f6c_dd1d);
        for case in 0..3000 {
            let n = if case % 4 == 0 { PAGE } else { rng.next() % 600 };
            let at = rng.next() % 64;
            let was: Vec<u8> = (0..n).map(|_| rng.next() as u8).collect();
            let mut now = was.clone();
            let density = 1 + rng.next() % 64;
            scribble(&mut rng, &mut now, density);
            let byte_runs = changed_runs(&now, &was);
            let want = crate::card_mask::dword_masks(at, &byte_runs);
            // entries already on the list never take this slice's bits
            let mut out = vec![(((at & !3) as u32), 0xdead_beef)];
            let (count, bytes) = masks(at, &now, &was, &mut out);
            assert_eq!(out[0], ((at & !3) as u32, 0xdead_beef), "case {case}");
            assert_eq!(out[1..], want[..], "case {case} at {at}");
            assert_eq!(count, byte_runs.len(), "case {case}");
            assert_eq!(bytes, byte_runs.iter().map(|r| r.1).sum::<usize>(), "case {case}");
        }
    }

    fn warp_buffer() -> Option<ID3D12Resource> {
        // SAFETY: plain DXGI and D3D12 creation calls on the software adapter.
        unsafe {
            let factory: IDXGIFactory4 = CreateDXGIFactory2(DXGI_CREATE_FACTORY_FLAGS(0)).ok()?;
            let adapter: IDXGIAdapter = factory.EnumWarpAdapter().ok()?;
            let mut device: Option<ID3D12Device> = None;
            D3D12CreateDevice(&adapter, D3D_FEATURE_LEVEL_11_0, &mut device).ok()?;
            let device = device?;
            let heap = D3D12_HEAP_PROPERTIES { Type: D3D12_HEAP_TYPE_DEFAULT, ..Default::default() };
            let desc = D3D12_RESOURCE_DESC {
                Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
                Width: 64 << 10,
                Height: 1,
                DepthOrArraySize: 1,
                MipLevels: 1,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
                ..Default::default()
            };
            let mut buffer: Option<ID3D12Resource> = None;
            device
                .CreateCommittedResource(&heap, D3D12_HEAP_FLAG_NONE, &desc, D3D12_RESOURCE_STATE_COMMON, None, &mut buffer)
                .ok()?;
            buffer
        }
    }

    #[derive(Clone, Copy, Debug)]
    enum Kind {
        Card,
        Masked,
        Snapshot,
        NoSnapshot,
    }

    #[derive(Debug, PartialEq)]
    struct Outcome {
        settle: bool,
        runs_left: usize,
        side: Vec<u8>,
        snapshot: Option<Vec<u8>>,
        copied: Vec<u64>,
        dirty: Vec<(usize, usize)>,
        pending: Vec<(u32, u32)>,
    }

    const PAGES: usize = 150;

    // title memory of PAGES pages starting part-way into a page, with page 7 unreadable
    struct Title {
        base: *mut u8,
        start: usize,
        len: usize,
    }

    impl Title {
        fn new() -> Title {
            let total = (PAGES + 2) * PAGE;
            // SAFETY: the test's own reservation.
            let base = unsafe { mem::VirtualAlloc(std::ptr::null(), total, mem::MEM_RESERVE | mem::MEM_COMMIT, mem::PAGE_READWRITE) } as *mut u8;
            assert!(!base.is_null());
            Title { base, start: base as usize + 0x1234, len: PAGES * PAGE - 0x2000 }
        }
        fn bytes(&self) -> &mut [u8] {
            // SAFETY: inside the reservation, committed read-write.
            unsafe { std::slice::from_raw_parts_mut(self.start as *mut u8, self.len) }
        }
        fn hide(&self, page: usize) {
            let mut old = 0;
            // SAFETY: a page of the reservation.
            unsafe { mem::VirtualProtect(self.base.add(page * PAGE).cast(), PAGE, mem::PAGE_NOACCESS, &mut old) };
        }
    }

    impl Drop for Title {
        fn drop(&mut self) {
            // SAFETY: the reservation made in `new`.
            unsafe { mem::VirtualFree(self.base.cast(), 0, mem::MEM_RELEASE) };
        }
    }

    // the plain merge (`wide` false) or the wide one, of the same title bytes over the same copy
    #[allow(clippy::too_many_arguments)]
    fn merge(
        buffer: &ID3D12Resource,
        title: &Title,
        held: &[u8],
        kind: Kind,
        ranges: &[(usize, usize)],
        runs_left: usize,
        wide: bool,
    ) -> Outcome {
        merge_timed(buffer, title, held, kind, ranges, runs_left, wide).0
    }

    #[allow(clippy::too_many_arguments)]
    fn merge_timed(
        buffer: &ID3D12Resource,
        title: &Title,
        held: &[u8],
        kind: Kind,
        ranges: &[(usize, usize)],
        runs_left: usize,
        wide: bool,
    ) -> (Outcome, f64) {
        merge_way(buffer, title, held, kind, ranges, runs_left, wide, false)
    }

    // the plain merge the byte way or the dense way (`dense`), or the wide one
    #[allow(clippy::too_many_arguments)]
    fn merge_way(
        buffer: &ID3D12Resource,
        title: &Title,
        held: &[u8],
        kind: Kind,
        ranges: &[(usize, usize)],
        runs_left: usize,
        wide: bool,
        dense: bool,
    ) -> (Outcome, f64) {
        DENSE.with(|d| d.set(dense));
        let mut side = held.to_vec();
        let mut shadow = Shadow::new(buffer.clone(), side.as_mut_ptr(), 0, title.start as u64, title.len as u64);
        let card = matches!(kind, Kind::Card | Kind::Masked);
        if card {
            shadow.card = Some(Card {
                staging: buffer.clone(),
                before: Mutex::new(None),
                dirty: Mutex::default(),
                sent: Mutex::default(),
                masked: Mutex::default(),
                size: 0,
                listed: Default::default(),
            });
        } else if matches!(kind, Kind::Snapshot) {
            *shadow.before.lock().unwrap() = Some(held.to_vec());
        }
        {
            let mut copied = shadow.copied.lock().unwrap();
            for (i, c) in copied.iter_mut().enumerate() {
                *c = (i % 3) as u64;
            }
        }
        let base = title.start / PAGE;
        let mut left = runs_left;
        let mut settle = false;
        let mut scratch = vec![0u8; PAGE];
        let t = std::time::Instant::now();
        for &(a, e) in ranges {
            let stale: Vec<usize> = (a / PAGE..e.div_ceil(PAGE)).filter(|&p| p >= base && p % 5 != 3).collect();
            let masked = matches!(kind, Kind::Masked);
            settle = if wide {
                shadow.merge_stale_wide(a, e, &stale, 9, &mut left, masked)
            } else {
                shadow.merge_stale(a, e, &stale, 9, &mut left, masked, &mut scratch)
            };
            if settle {
                break;
            }
        }
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        DENSE.with(|d| d.set(false));
        let card = shadow.card.take();
        let out = Outcome {
            settle,
            runs_left: left,
            side: side.clone(),
            snapshot: shadow.before.lock().unwrap().clone(),
            copied: shadow.copied.lock().unwrap().clone(),
            dirty: card.as_ref().map(|c| c.dirty.lock().unwrap().clone()).unwrap_or_default(),
            pending: card.as_ref().map(|c| c.masked.lock().unwrap().pending.clone()).unwrap_or_default(),
        };
        drop(shadow);
        (out, ms)
    }

    #[test]
    fn the_wide_merge_puts_what_the_plain_merge_puts() {
        let Some(buffer) = warp_buffer() else {
            return;
        };
        let title = Title::new();
        let mut rng = Rng(0x51_7cc1_b727_220a);
        let held: Vec<u8> = (0..title.len).map(|_| rng.next() as u8).collect();
        title.bytes().copy_from_slice(&held);
        scribble(&mut rng, title.bytes(), 40_000);
        // a busy page rewritten whole, and one never written
        title.bytes()[20 * PAGE..21 * PAGE].iter_mut().for_each(|b| *b = !*b);
        title.hide(7);
        let s = title.start;
        let ranges = [(s + 100, s + 3 * PAGE + 77), (s + 5 * PAGE, s + 140 * PAGE), (s + 2 * PAGE, s + 9 * PAGE + 1)];
        let mut checked = 0;
        for kind in [Kind::Card, Kind::Masked, Kind::Snapshot, Kind::NoSnapshot] {
            for runs_left in [usize::MAX, 1 << 30, 50_000, 3_000, 0] {
                let plain = merge(&buffer, &title, &held, kind, &ranges, runs_left, false);
                let wide = merge(&buffer, &title, &held, kind, &ranges, runs_left, true);
                assert!(plain == wide, "{kind:?} with {runs_left} runs: settle {} vs {}, left {} vs {}", plain.settle, wide.settle, plain.runs_left, wide.runs_left);
                checked += usize::from(plain.side != held || plain.snapshot.as_deref().is_some_and(|b| b != &held[..]));
            }
        }
        // the cases did merge something, and the caps did cut some
        assert!(checked > 5, "{checked}");
        let plain = merge(&buffer, &title, &held, Kind::Card, &ranges, 3_000, false);
        assert!(plain.settle);
    }

    /// VICTORY_MERGE_DENSE: the dense way puts, notes and counts what the
    /// byte way does, over pages sparse and dense, whole pages rewritten,
    /// ranges that end at and inside pages, and every kind of copy.
    #[test]
    fn the_dense_merge_puts_what_the_byte_merge_puts() {
        let Some(buffer) = warp_buffer() else {
            return;
        };
        let title = Title::new();
        let mut rng = Rng(0x0dd5_eed5_1234_8765);
        let held: Vec<u8> = (0..title.len).map(|_| rng.next() as u8).collect();
        title.bytes().copy_from_slice(&held);
        // per page: untouched, a few runs, or dense (about 1,400 a page, as fast-15s)
        for page in 0..PAGES - 2 {
            let density = match page % 4 {
                0 => 0,
                1 => 1 + rng.next() % 8,
                2 => 1 + rng.next() % 200,
                _ => 1_400,
            };
            let lo = page * PAGE;
            scribble(&mut rng, &mut title.bytes()[lo..lo + PAGE], density);
        }
        // a page rewritten whole, the first and last bytes of pages, and one never readable
        title.bytes()[30 * PAGE..31 * PAGE].iter_mut().for_each(|b| *b = !*b);
        for page in [40, 41, 42] {
            title.bytes()[page * PAGE] ^= 0x5a;
            title.bytes()[page * PAGE - 1] ^= 0xa5;
        }
        title.hide(7);
        let s = title.start;
        let ranges = [
            (s, s + 3 * PAGE + 77),
            (s + 5 * PAGE - 1, s + 60 * PAGE + 1),
            (s + 2 * PAGE + 3, s + 9 * PAGE),
            (s + 61 * PAGE, s + title.len),
        ];
        let mut cut = 0;
        for kind in [Kind::Card, Kind::Masked, Kind::Snapshot, Kind::NoSnapshot] {
            for runs_left in [usize::MAX, 200_000, 20_000, 700, 0] {
                let (byte, _) = merge_way(&buffer, &title, &held, kind, &ranges, runs_left, false, false);
                let (dense, _) = merge_way(&buffer, &title, &held, kind, &ranges, runs_left, false, true);
                assert!(
                    byte == dense,
                    "{kind:?} with {runs_left} runs: settle {} vs {}, left {} vs {}, dirty {} vs {}, pending {} vs {}",
                    byte.settle,
                    dense.settle,
                    byte.runs_left,
                    dense.runs_left,
                    byte.dirty.len(),
                    dense.dirty.len(),
                    byte.pending.len(),
                    dense.pending.len()
                );
                cut += usize::from(byte.settle);
            }
        }
        // the caps did cut some
        assert!(cut >= 3, "{cut}");
        let (byte, _) = merge_way(&buffer, &title, &held, Kind::Masked, &ranges, 0, false, false);
        assert!(!byte.pending.is_empty() && !byte.settle, "masked takes pages past the run cap");
    }

    // timings for the report: cargo test --release merge_timings -- --ignored --nocapture
    #[test]
    #[ignore]
    fn merge_timings() {
        let Some(buffer) = warp_buffer() else {
            return;
        };
        let title = Title::new();
        let mut rng = Rng(0x1234_5678_9abc_def1);
        let held: Vec<u8> = (0..title.len).map(|_| rng.next() as u8).collect();
        title.bytes().copy_from_slice(&held);
        // about 2,000 runs a page, as fast-8's late frames (840k runs over 400 pages)
        scribble(&mut rng, title.bytes(), 2_000 * PAGES);
        let s = title.start;
        let ranges = [(s, s + title.len)];
        let mut page_now = vec![0u8; PAGE];
        page_now.copy_from_slice(&title.bytes()[..PAGE]);
        let page_was = &held[..PAGE];
        let time = |f: &mut dyn FnMut()| {
            let t = std::time::Instant::now();
            for _ in 0..20 {
                f();
            }
            t.elapsed().as_secs_f64() * 1000.0 / 20.0
        };
        let byte_runs = time(&mut || {
            std::hint::black_box(changed_runs(&page_now, page_was));
        });
        let word_runs = time(&mut || {
            std::hint::black_box(runs(&page_now, page_was));
        });
        let byte_masks = time(&mut || {
            std::hint::black_box(crate::card_mask::dword_masks(0, &changed_runs(&page_now, page_was)));
        });
        let word_masks = time(&mut || {
            let mut out = Vec::new();
            std::hint::black_box(masks(0, &page_now, page_was, &mut out));
        });
        println!("one 64 KiB page, {} runs: changed_runs {byte_runs:.3} ms, word runs {word_runs:.3} ms; masks byte {byte_masks:.3} ms, word {word_masks:.3} ms", changed_runs(&page_now, page_was).len());
        for kind in [Kind::Card, Kind::Masked, Kind::Snapshot] {
            let (mut plain, mut wide, mut dense) = (0.0, 0.0, 0.0);
            for _ in 0..5 {
                let (p, pt) = merge_timed(&buffer, &title, &held, kind, &ranges, usize::MAX, false);
                let (w, wt) = merge_timed(&buffer, &title, &held, kind, &ranges, usize::MAX, true);
                let (d, dt) = merge_way(&buffer, &title, &held, kind, &ranges, usize::MAX, false, true);
                (plain, wide, dense) = (plain + pt / 5.0, wide + wt / 5.0, dense + dt / 5.0);
                assert!(p == w && p == d);
            }
            println!("{kind:?}: {} pages: plain {plain:.1} ms, wide {wide:.1} ms, dense {dense:.1} ms (merge only)", PAGES * 4 / 5);
        }
    }
}
