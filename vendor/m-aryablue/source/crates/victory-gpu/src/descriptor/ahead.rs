//! heap records mirrored ahead of the submit thread (VICTORY_HEAP_SYNC_AHEAD=1, off by default)

use super::{SHADER_HEAPS, SYNCING, heap_parts, sync_range};
use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering::Relaxed};
use std::sync::{Condvar, Mutex};

// whether a helper thread mirrors changed heap pages ahead of submissions
pub(crate) fn on() -> bool {
    // live: off, the helper gets no kicks and waits; what it took is drained by the next pass
    static S: crate::switch::Switch = crate::switch::Switch::new("HEAP_SYNC_AHEAD");
    S.on()
}

// how far into the gap between submit passes the helper takes its pages, in percent (VICTORY_HEAP_SYNC_AHEAD_DELAY, default 50)
fn delay_percent() -> u64 {
    static PCT: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *PCT.get_or_init(|| std::env::var("VICTORY_HEAP_SYNC_AHEAD_DELAY").ok().and_then(|v| v.trim().parse().ok()).unwrap_or(50).min(100))
}

// a page of a heap: heap, its start, title range
type Page = (usize, u64, usize, usize);

// taken pages, only touched under SYNCING: for the helper, for the submit pass alone, and pages whose last look mirrored nothing
struct Pages {
    helper: Vec<Page>,
    submit: Vec<Page>,
    barren: Option<HashSet<usize>>,
}

static PAGES: Mutex<Pages> = Mutex::new(Pages { helper: Vec::new(), submit: Vec::new(), barren: None });

fn pages() -> std::sync::MutexGuard<'static, Pages> {
    PAGES.lock().unwrap_or_else(|e| e.into_inner())
}

// submit passes waiting for or holding SYNCING: the helper stays off it meanwhile
static WANTED: AtomicUsize = AtomicUsize::new(0);

// submit passes ended (the helper's kick), and the last gap between two in us
static KICK: (Mutex<u64>, Condvar) = (Mutex::new(0), Condvar::new());
static GAP: AtomicU64 = AtomicU64::new(0);

// pages the helper mirrors under one taking of SYNCING
const CHUNK: usize = 2;

// since the last line: helper pages, records mirrored (= its made-lock takings), helper us, submit passes, us they waited,
// pages they drained, pages left to them as barren, helper passes, pages a page-wide lock would have taken
static STATS: [AtomicU64; 9] = [const { AtomicU64::new(0) }; 9];

// a submit pass is about to take SYNCING: starts the helper on first use
pub(super) fn submit_begin() -> Option<std::time::Instant> {
    if !on() {
        return None;
    }
    static STARTED: std::sync::Once = std::sync::Once::new();
    STARTED.call_once(|| {
        let _ = std::thread::Builder::new().name("victory-heap-ahead".into()).spawn(|| {
            crate::placement::helper();
            helper()
        });
    });
    WANTED.fetch_add(1, Relaxed);
    BEGUN.set(true);
    Some(std::time::Instant::now())
}

thread_local! {
    // this thread's pass counted itself in WANTED: its end takes it out, whatever the switch says by then
    static BEGUN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

// the submit pass holds SYNCING now
pub(super) fn submit_waited(asking: Option<std::time::Instant>) {
    if let Some(at) = asking {
        STATS[3].fetch_add(1, Relaxed);
        STATS[4].fetch_add(at.elapsed().as_micros() as u64, Relaxed);
    }
}

// the submit pass is done: the helper is kicked for the next gap
pub(super) fn submit_end() {
    if !BEGUN.replace(false) {
        return;
    }
    WANTED.fetch_sub(1, Relaxed);
    static LAST: Mutex<Option<std::time::Instant>> = Mutex::new(None);
    let now = std::time::Instant::now();
    if let Some(last) = LAST.lock().unwrap_or_else(|e| e.into_inner()).replace(now) {
        GAP.store(now.duration_since(last).as_micros() as u64, Relaxed);
    }
    *KICK.0.lock().unwrap_or_else(|e| e.into_inner()) += 1;
    KICK.1.notify_all();
}

// under SYNCING: mirrors every page still queued for the helper or left to the submit pass (heaps gone since are skipped)
pub(super) fn drain(heaps: &[(u64, u64, usize)], page_bytes: &mut Vec<u8>, counts: &mut [u64; 3]) {
    let (helper, submit) = {
        let mut p = pages();
        (std::mem::take(&mut p.helper), std::mem::take(&mut p.submit))
    };
    STATS[5].fetch_add((helper.len() + submit.len()) as u64, Relaxed);
    run(heaps, &helper, page_bytes, counts, false);
    run(heaps, &submit, page_bytes, counts, false);
}

// mirrors `list` page by page; a page that mirrors nothing is barren for the helper's next take
fn run(heaps: &[(u64, u64, usize)], list: &[Page], page_bytes: &mut Vec<u8>, counts: &mut [u64; 3], fine: bool) {
    for &(at, start, from, to) in list {
        if !heaps.iter().any(|&(s, _, h)| h == at && s == start) {
            continue;
        }
        let Some((parts, _)) = heap_parts(at, start) else {
            continue;
        };
        let before = counts[2];
        sync_range(&parts, (from, to), page_bytes, counts, fine);
        let mut p = pages();
        let barren = p.barren.get_or_insert_with(HashSet::new);
        if counts[2] == before {
            barren.insert(from);
        } else {
            barren.remove(&from);
            if fine {
                STATS[8].fetch_add(1, Relaxed);
            }
        }
    }
}

// under SYNCING: every heap's changed pages, barren ones left to the submit pass
fn take(heaps: &[(u64, u64, usize)]) {
    let mut p = pages();
    let mut left = 0;
    for &(start, end, at) in heaps {
        let Some((_, written)) = heap_parts(at, start) else {
            continue;
        };
        for (from, to) in written.take(start as usize, end as usize) {
            // page by page, so a step can stop between them
            let mut page = from & !(crate::watch::PAGE - 1);
            while page < to {
                let piece = (at, start, page.max(from), (page + crate::watch::PAGE).min(to));
                if p.barren.as_ref().is_some_and(|b| b.contains(&piece.2)) {
                    p.submit.push(piece);
                    left += 1;
                } else {
                    p.helper.push(piece);
                }
                page += crate::watch::PAGE;
            }
        }
    }
    STATS[6].fetch_add(left, Relaxed);
}

// one helper step: takes the changed pages first if `fresh` and none are queued, then mirrors up to `chunk` of them
// record by record. Whether it mirrored a page
pub(super) fn step(page_bytes: &mut Vec<u8>, chunk: usize, fresh: bool) -> bool {
    let started = std::time::Instant::now();
    let _one = SYNCING.lock().unwrap_or_else(|e| e.into_inner());
    let heaps = SHADER_HEAPS.read().unwrap_or_else(|e| e.into_inner());
    let empty = pages().helper.is_empty();
    if fresh && empty {
        take(&heaps);
    }
    let now: Vec<Page> = {
        let mut p = pages();
        let n = p.helper.len().min(chunk);
        p.helper.drain(..n).collect()
    };
    let mut counts = [0u64; 3];
    run(&heaps, &now, page_bytes, &mut counts, true);
    STATS[0].fetch_add(counts[0], Relaxed);
    STATS[1].fetch_add(counts[2], Relaxed);
    STATS[2].fetch_add(started.elapsed().as_micros() as u64, Relaxed);
    !now.is_empty()
}

// once per gap between submit passes: a kick, a share of the last gap, then one take mirrored a chunk at a time,
// off SYNCING while a submit pass wants it
fn helper() {
    let mut page_bytes = Vec::new();
    let mut seen = 0;
    loop {
        {
            let mut kicks = KICK.0.lock().unwrap_or_else(|e| e.into_inner());
            while *kicks == seen {
                kicks = KICK.1.wait(kicks).unwrap_or_else(|e| e.into_inner());
            }
            seen = *kicks;
        }
        let wait = GAP.load(Relaxed) * delay_percent() / 100;
        std::thread::sleep(std::time::Duration::from_micros(wait.min(50_000)));
        if WANTED.load(Relaxed) > 0 {
            continue;
        }
        STATS[7].fetch_add(1, Relaxed);
        let mut fresh = true;
        while WANTED.load(Relaxed) == 0 && step(&mut page_bytes, CHUNK, fresh) {
            fresh = false;
        }
    }
}

// with the frame line
pub(super) fn line(frames: f64) {
    if !on() {
        return;
    }
    let s: Vec<f64> = STATS.iter().map(|s| s.swap(0, Relaxed) as f64 / frames).collect();
    tracing::info!(
        "heap records mirrored ahead per frame (VICTORY_HEAP_SYNC_AHEAD=1, delay {}% of the gap): the helper made {:.1} passes, \
         {:.1} pages, {:.1} records mirrored (its made-lock takings; {:.1} with one per page), {:.2} ms; {:.1} pages left to \
         submit passes as barren; {:.1} submit passes waited {:.2} ms for it and mirrored {:.1} pages it had taken or left",
        delay_percent(),
        s[7],
        s[0],
        s[1],
        s[8],
        s[2] / 1000.0,
        s[6],
        s[3],
        s[4] / 1000.0,
        s[5],
    );
}

#[cfg(test)]
pub(super) mod tests {
    use super::super::*;
    use crate::object::{S_OK, slot};
    use super::{pages, step};

    // one heap-sync test at a time: each pass takes every heap's pages, and the queues and barren notes are shared
    pub(crate) static TESTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

    // this test's turn, with no barren notes of earlier tests' heaps (addresses come round again)
    fn fresh_notes() -> std::sync::MutexGuard<'static, ()> {
        let turn = TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let mut p = pages();
        p.barren = None;
        p.helper.clear();
        p.submit.clear();
        turn
    }
    use windows_sys::Win32::System::Memory::{MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE, VirtualAlloc, VirtualFree};

    const MEM_WRITE_WATCH: u32 = 0x20_0000;

    #[repr(C)]
    struct Extended(HeapDesc, Placement);

    fn create(desc: HeapDesc, placement: Placement, out: &mut This) -> crate::object::HRESULT {
        let extended = Extended(desc, placement);
        create_heap(std::ptr::null(), &extended.0, &HEAP_IIDS[0], out)
    }

    // a placed shader-visible sampler heap of 64 slots in write-watched memory
    fn sampler_heap() -> (*mut std::ffi::c_void, This) {
        // SAFETY: a fresh allocation, released by the test
        let memory = unsafe { VirtualAlloc(std::ptr::null(), crate::watch::PAGE, MEM_RESERVE | MEM_COMMIT | MEM_WRITE_WATCH, PAGE_READWRITE) };
        assert!(!memory.is_null());
        let desc = HeapDesc { kind: SAMPLERS, count: 64, flags: SHADER_VISIBLE, nodes: 1 };
        let placement = Placement { address: memory as u64, size: 64 * 16 };
        let mut h: This = std::ptr::null();
        assert_eq!(create(desc, placement, &mut h), S_OK);
        (memory, h)
    }

    fn release(memory: *mut std::ffi::c_void, h: This) {
        slot::<extern "system" fn(This) -> u32>(h, 0x10)(h);
        // SAFETY: the test's allocation
        unsafe { VirtualFree(memory, 0, MEM_RELEASE) };
    }

    // a sampler record made elsewhere, as the title makes one before copying it
    fn sampler(filter: u32) -> [u64; 2] {
        let mut record = [0u64; 2];
        let words = [filter, 3, 3, 3, 0, 1, 0, 0, 0, 0, 0, 0x7f7f_ffff, 0];
        create_sampler(std::ptr::null(), &words, record.as_mut_ptr() as u64);
        record
    }

    // the title copies a record into slot `index` as bytes
    fn copy_in(memory: *mut std::ffi::c_void, index: usize, record: &[u64; 2]) {
        // SAFETY: the slot and the record both hold 16 bytes
        unsafe { std::ptr::copy_nonoverlapping(record.as_ptr().cast::<u8>(), memory.cast::<u8>().add(index * 16), 16) };
    }

    // a record written after the helper's pass is mirrored by the submit pass
    #[test]
    fn a_record_written_after_the_helper_is_mirrored_at_submit() {
        let _turn = fresh_notes();
        let (memory, h) = sampler_heap();
        if let Some(host) = heap(h).host() {
            let (a, b) = (sampler(21), sampler(5));
            copy_in(memory, 5, &a);
            let mut bytes = Vec::new();
            while step(&mut bytes, 2, true) {}
            assert!(host.made(5), "the helper mirrored nothing");
            copy_in(memory, 9, &b);
            assert!(!host.made(9));
            sync_heaps();
            assert!(host.made(9), "the submit pass missed a record written after the helper");
        }
        release(memory, h);
    }

    // pages the helper took and had not mirrored yet are mirrored by the submit pass
    #[test]
    fn pages_the_helper_took_are_mirrored_at_submit() {
        let _turn = fresh_notes();
        let (memory, h) = sampler_heap();
        if let Some(host) = heap(h).host() {
            copy_in(memory, 7, &sampler(21));
            let mut bytes = Vec::new();
            // taken from the write watch, none mirrored
            assert!(!step(&mut bytes, 0, true));
            assert!(!host.made(7));
            sync_heaps();
            assert!(host.made(7), "a page the helper took was lost");
        }
        release(memory, h);
    }

    // a page that gave the helper nothing last time is left to the submit pass, which mirrors it in the same submission
    #[test]
    fn a_barren_page_is_mirrored_by_the_same_submit_pass() {
        let _turn = fresh_notes();
        let (memory, h) = sampler_heap();
        if let Some(host) = heap(h).host() {
            let (a, b) = (sampler(21), sampler(5));
            let mut bytes = Vec::new();
            copy_in(memory, 5, &a);
            while step(&mut bytes, 2, true) {}
            assert!(host.made(5));
            // written again with the same record: the helper mirrors nothing there
            copy_in(memory, 5, &a);
            while step(&mut bytes, 2, true) {}
            copy_in(memory, 9, &b);
            // taken, left to the submit pass
            assert!(!step(&mut bytes, 2, true));
            assert!(!host.made(9), "the helper mirrored a barren page");
            sync_heaps();
            assert!(host.made(9), "the submit pass missed a page left to it");
        }
        release(memory, h);
    }

    // the same writes with and without the helper's passes leave the same host slots
    #[test]
    fn host_heaps_match_with_and_without_the_helper() {
        let (a, b, c) = (sampler(21), sampler(5), sampler(1));
        let run = |helper: bool| -> Option<Vec<u64>> {
            let _turn = fresh_notes();
        let (memory, h) = sampler_heap();
            let mut keys = None;
            if let Some(host) = heap(h).host() {
                let mut bytes = Vec::new();
                copy_in(memory, 1, &a);
                copy_in(memory, 2, &b);
                if helper {
                    while step(&mut bytes, 2, true) {}
                }
                copy_in(memory, 3, &c);
                copy_in(memory, 1, &c);
                if helper {
                    // taken, left for the submit pass
                    step(&mut bytes, 0, true);
                }
                copy_in(memory, 2, &[0, 0]);
                copy_in(memory, 4, &a);
                sync_heaps();
                keys = Some((0..64).map(|i| host.made_key(i)).collect());
            }
            release(memory, h);
            keys
        };
        let (Some(off), Some(on)) = (run(false), run(true)) else {
            return;
        };
        assert!(off.iter().filter(|&&k| k != 0).count() >= 3, "nothing made");
        assert_eq!(off, on);
    }
}
