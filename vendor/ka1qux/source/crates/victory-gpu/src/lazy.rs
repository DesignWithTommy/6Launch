//! What shaders wrote, put into title memory only when something needs it
//! (`VICTORY_LAZY_BACK=1`; off by default).
//!
//! After a submission the copy-back puts the bytes shaders wrote into
//! title memory ([`crate::shadow::Shadow::copy_back_with`] and the merges
//! of the relay and of copies in the card's memory). A census of the
//! title's first touches of those pages (victory-fork issue #1169, after a
//! diagnostic by Yorivel) found the title's processor touching 0.1-0.3 % of
//! them: nearly everything written was read again only by our own next
//! refresh, which took our write for a title write.
//!
//! With the switch, a copy-back that would write a 4 KiB page of private
//! title memory keeps the page here instead: the page as title memory
//! holds it, with the GPU's bytes put in, and a mask of the bytes the
//! copy-back would have written. Title memory itself is made
//! `PAGE_NOACCESS`, so nothing in user mode can see its older bytes:
//!
//! - the title's processor (or ours, anywhere) touching the page faults
//!   into our vectored handler ([`fault`], called first by `crate::watch`'s
//!   handler and by the two in-place copy handlers), which writes the
//!   masked bytes in, gives the page its protection back and lets the
//!   access go on;
//! - our own reads of title memory ([`crate::host`]'s `read`, `read_into`,
//!   `copy_mapped`) take a pending page's bytes from here ([`read_over`])
//!   and trip nothing: a refresh of a host copy over a pending page, and
//!   the next copy-back's merge of the same page, see what title memory
//!   would hold;
//! - a later copy-back of the same page puts its bytes in here too;
//! - kernel access does not fault, it fails: every write into title
//!   memory the runtime announces (`crate::watch::writing_held`,
//!   `writing_async`: files, DirectStorage, our copies into title memory
//!   and the eager copy-back itself) and every kernel read it announces
//!   ([`kernel_will_read`]) puts the pending pages there back first and
//!   keeps new ones from being made there while the access lasts
//!   ([`hold`], [`pin`]);
//! - memory that changes meaning (commits at an address, frees, remaps,
//!   heaps opened on title memory) is put back or let go first.
//!
//! Which pages: whole 4 KiB pages inside the title buffer whose copy wrote
//! them (never a window outside buffers, whose pages may hold anything),
//! not view memory (`crate::inplace::cover`: physical pages are seen at
//! other addresses this cannot protect), not in a heap opened on title
//! memory, read-write when taken, not written by the title's processor in
//! the last two seconds (a page the title also writes costs two faults a
//! frame here, more than writing it). Anything else is written as before.
//!
//! Everything this module does under its lock touches only its own
//! memory (taken from Windows, never the heap) and the pages it protects:
//! the fault handler may run on a thread that holds the process heap's
//! lock.
//!
//! Known limits: a kernel access the runtime does not announce (a system
//! call the title makes itself with a pending page as its buffer, a
//! `VirtualProtect` or `VirtualFree` of its own on one) fails or sees the
//! older bytes; pages freed and remapped behind our back are caught when
//! they are next put back (their protection is no longer ours) and, for
//! our reads, by a look at their protection at most every 100 ms.

use std::cell::Cell;
use std::sync::Mutex;
use std::sync::atomic::{AtomicPtr, AtomicU32, AtomicU64, Ordering::Acquire, Ordering::Relaxed, Ordering::Release};

use windows_sys::Win32::System::Memory::{
    MEM_COMMIT, MEM_PRIVATE, MEM_RESERVE, MEMORY_BASIC_INFORMATION, PAGE_NOACCESS, PAGE_READONLY, PAGE_READWRITE,
    VirtualAlloc, VirtualProtect, VirtualQuery,
};

/// The unit kept and protected: the kernel's page.
pub(crate) const PAGE: usize = 4096;
/// Words of a page's mask (one bit a byte).
const WORDS: usize = PAGE / 64;
/// Pages a second-level table covers: a GiB.
const GIB_PAGES: usize = 1 << 18;
/// Second-level tables: the 128 TiB of user address space.
const TOP: usize = 1 << 17;

/// What is pending in a GiB of address space: a bit a page, and the slot
/// (plus one) that keeps it. Taken from Windows when first needed, never
/// given back (a megabyte a GiB that ever held a pending page).
#[repr(C)]
struct Gib {
    bits: [AtomicU64; GIB_PAGES / 64],
    slots: [AtomicU32; GIB_PAGES],
}

static MAP: [AtomicPtr<Gib>; TOP] = [const { AtomicPtr::new(std::ptr::null_mut()) }; TOP];

/// Pending pages now.
static LIVE: AtomicU64 = AtomicU64::new(0);

/// A kept page.
#[repr(C)]
struct Slot {
    /// The page as title memory holds it, with what copy-backs put in.
    bytes: [u8; PAGE],
    /// The bytes copy-backs put in: what goes into title memory.
    mask: [u64; WORDS],
    /// Its page number.
    page: usize,
    /// When a copy-back last put bytes in (the oldest go first when full).
    seq: u64,
    /// When its protection was last found ours (milliseconds).
    checked: u64,
    /// Its protection before (read-write).
    protect: u32,
}

/// Kernel accesses under way that pending pages must not be made over:
/// holds (until dropped) and pins (until a moment).
const HOLDS: usize = 256;
const PINS: usize = 256;
/// Heaps opened on title memory (the card reads title memory itself there).
const PLACES: usize = 256;
/// Slots committed at a time.
const CHUNK: usize = 256;

struct State {
    slots: *mut Slot,
    cap: usize,
    /// Slots ever used (all below are committed).
    fresh: usize,
    committed: usize,
    /// Slots given back (capacity `cap`: pushing never allocates).
    free: Vec<u32>,
    seq: u64,
    holds: [(usize, usize, u64); HOLDS],
    holds_n: usize,
    /// Holds that did not fit: while any lasts, nothing is made pending.
    holds_over: usize,
    next_hold: u64,
    pins: [(usize, usize, u64); PINS],
    pins_n: usize,
    places: [(usize, usize); PLACES],
    places_n: usize,
    places_over: bool,
}

// SAFETY: the raw slot memory is ours, and only touched under the lock.
unsafe impl Send for State {}

static STATE: Mutex<Option<State>> = Mutex::new(None);

thread_local! {
    /// This thread holds [`STATE`]'s lock: a fault it takes meanwhile is
    /// not ours to take (the lock is not re-entrant).
    static HOLDING: Cell<bool> = const { Cell::new(false) };
    /// Tests turn the switch on for their own thread.
    static TEST_ON: Cell<bool> = const { Cell::new(false) };
}

struct Locked(std::sync::MutexGuard<'static, Option<State>>);

impl Drop for Locked {
    fn drop(&mut self) {
        HOLDING.set(false);
    }
}

impl std::ops::Deref for Locked {
    type Target = Option<State>;
    fn deref(&self) -> &Option<State> {
        &self.0
    }
}

impl std::ops::DerefMut for Locked {
    fn deref_mut(&mut self) -> &mut Option<State> {
        &mut self.0
    }
}

fn lock() -> Locked {
    let g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    HOLDING.set(true);
    Locked(g)
}

/// Whether copy-backs leave pages pending (`VICTORY_LAZY_BACK=1`).
pub(crate) fn on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        let on = std::env::var_os("VICTORY_LAZY_BACK").is_some_and(|v| v == "1");
        if on {
            tracing::info!(
                "what shaders wrote is put into title memory only when something needs it (VICTORY_LAZY_BACK=1): up to {} MB \
                 kept, pages the title's processor wrote in the last {HOT_MS} ms written at once",
                (cap_pages() * PAGE) >> 20
            );
        }
        on
    }) || TEST_ON.get()
}

/// [`on`], for the runtime (which host memory calls to stand in front of).
pub fn lazy_back_on() -> bool {
    on()
}

/// Whether the copy that wrote a pending page notes itself current there
/// (`VICTORY_LAZY_BACK_RETAKE=1`: no, its next refresh takes the page
/// again from what is kept here).
pub(crate) fn retake() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("VICTORY_LAZY_BACK_RETAKE").is_some_and(|v| v == "1"))
}

/// The most pages kept (`VICTORY_LAZY_BACK_MB`, 256).
fn cap_pages() -> usize {
    static N: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *N.get_or_init(|| {
        let mb: usize = std::env::var("VICTORY_LAZY_BACK_MB").ok().and_then(|v| v.trim().parse().ok()).unwrap_or(256);
        (mb.clamp(1, 16384) << 20) / PAGE
    })
}

/// A page the title's processor touched within this is written at once.
const HOT_MS: u64 = 2000;
/// A pending page's protection is looked at again by our reads after this.
const CHECK_MS: u64 = 100;
/// A kernel read announced with [`kernel_will_read`] may still run this long.
const PIN_MS: u64 = 2000;

fn now_ms() -> u64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START.get_or_init(std::time::Instant::now).elapsed().as_millis() as u64 + 1
}

/// Pages put back on a fault or otherwise lately, by the low bits of their
/// number: a fault that raced with another thread's putting back is
/// retried, and a page the title touches is not made pending again for a
/// while.
static RECENT: [AtomicU64; 4096] = [const { AtomicU64::new(0) }; 4096];
static RECENT_AT: [AtomicU64; 4096] = [const { AtomicU64::new(0) }; 4096];
static HOT: [AtomicU64; 4096] = [const { AtomicU64::new(0) }; 4096];
static HOT_AT: [AtomicU64; 4096] = [const { AtomicU64::new(0) }; 4096];

fn note_in(table: &[AtomicU64; 4096], at: &[AtomicU64; 4096], page: usize) {
    let i = page & 4095;
    table[i].store(page as u64 + 1, Relaxed);
    at[i].store(now_ms(), Relaxed);
}

fn noted_in(table: &[AtomicU64; 4096], at: &[AtomicU64; 4096], page: usize, ms: u64) -> bool {
    let i = page & 4095;
    table[i].load(Relaxed) == page as u64 + 1 && now_ms().saturating_sub(at[i].load(Relaxed)) < ms
}

/// What is counted (per frame line and since the start).
#[derive(Clone, Copy)]
#[repr(usize)]
enum C {
    /// Pages made pending.
    New,
    /// Copy-backs into pages already pending (no system call).
    Again,
    /// Bytes not written into title memory by the copy-back.
    Saved,
    /// Pages put back on a fault (the title's processor, or ours).
    Fault,
    /// Pages put back before a kernel write or an eager write.
    Kernel,
    /// Pages put back before a kernel read.
    Read,
    /// Pages put back before memory changed meaning (a commit, a heap on it).
    Remap,
    /// Pages put back for room.
    Cap,
    /// Pages let go without being put back (memory freed or remapped).
    Gone,
    /// Kept pages found no longer ours (freed behind our back): let go.
    Stale,
    /// Bytes written into title memory by putting pages back.
    Written,
    /// Nanoseconds in the fault handler putting pages back.
    HandlerNs,
    /// Pages our reads took from here.
    Served,
    /// Pages left to the copy-back: written by the title lately.
    RefuseHot,
    /// ... not whole inside the copy's title buffer, or view memory, or in a
    /// window, or in a heap opened on title memory.
    RefuseWhere,
    /// ... under a kernel access.
    RefuseHeld,
    /// ... not read-write, or Windows refused the protection.
    RefuseProtect,
    /// ... no room.
    RefuseFull,
}

const COUNTS: usize = C::RefuseFull as usize + 1;
static COUNT: [AtomicU64; COUNTS] = [const { AtomicU64::new(0) }; COUNTS];
static TOTAL: [AtomicU64; COUNTS] = [const { AtomicU64::new(0) }; COUNTS];

fn count(c: C, n: u64) {
    COUNT[c as usize].fetch_add(n, Relaxed);
    TOTAL[c as usize].fetch_add(n, Relaxed);
}

/// Why pending pages are put back.
#[derive(Clone, Copy)]
enum Why {
    Fault,
    Kernel,
    Read,
    Remap,
    Cap,
}

impl Why {
    fn counted(self) -> C {
        match self {
            Why::Fault => C::Fault,
            Why::Kernel => C::Kernel,
            Why::Read => C::Read,
            Why::Remap => C::Remap,
            Why::Cap => C::Cap,
        }
    }
}

fn gib(page: usize) -> Option<&'static Gib> {
    let ptr = MAP.get(page / GIB_PAGES)?.load(Acquire);
    // SAFETY: tables are never given back once published.
    (!ptr.is_null()).then(|| unsafe { &*ptr })
}

/// [`gib`], made if missing (under the lock).
fn gib_made(page: usize) -> Option<&'static Gib> {
    if let Some(g) = gib(page) {
        return Some(g);
    }
    let place = MAP.get(page / GIB_PAGES)?;
    // SAFETY: fresh zeroed memory of ours (all-zero atomics are valid).
    let ptr = unsafe { VirtualAlloc(std::ptr::null(), size_of::<Gib>(), MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE) };
    if ptr.is_null() {
        return None;
    }
    place.store(ptr.cast(), Release);
    // SAFETY: just published, never given back.
    Some(unsafe { &*ptr.cast::<Gib>() })
}

/// Whether page `page` is pending (no lock: exact under it).
fn pending(page: usize) -> bool {
    gib(page).is_some_and(|g| {
        let i = page % GIB_PAGES;
        g.bits[i / 64].load(Acquire) >> (i % 64) & 1 != 0
    })
}

/// Calls `f` with every pending page of `[first, last)` in order, each
/// word read once (`f` may change the bits).
fn each_pending(first: usize, last: usize, mut f: impl FnMut(usize) -> bool) {
    let mut page = first;
    while page < last {
        let chunk_end = ((page / GIB_PAGES + 1) * GIB_PAGES).min(last);
        let Some(g) = gib(page) else {
            page = chunk_end;
            continue;
        };
        while page < chunk_end {
            let i = page % GIB_PAGES;
            let word = i / 64;
            let lo = i % 64;
            let hi = (chunk_end - (page - lo)).min(64);
            let mut bits = g.bits[word].load(Acquire) >> lo << lo;
            if hi < 64 {
                bits &= (1u64 << hi) - 1;
            }
            while bits != 0 {
                let b = bits.trailing_zeros() as usize;
                bits &= bits - 1;
                if !f(page - lo + b) {
                    return;
                }
            }
            page = page - lo + 64;
        }
        page = chunk_end;
    }
}

/// Whether any byte of `[at, at + len)` is in a pending page.
pub(crate) fn touches(at: usize, len: usize) -> bool {
    if LIVE.load(Relaxed) == 0 || len == 0 {
        return false;
    }
    let mut any = false;
    each_pending(at / PAGE, at.saturating_add(len).div_ceil(PAGE), |_| {
        any = true;
        false
    });
    any
}

impl State {
    fn new() -> Option<State> {
        let cap = cap_pages();
        // Reserved now, committed as used.
        // SAFETY: a plain reservation of ours.
        let slots = unsafe { VirtualAlloc(std::ptr::null(), cap * size_of::<Slot>(), MEM_RESERVE, PAGE_READWRITE) };
        if slots.is_null() {
            tracing::warn!(cap, "VICTORY_LAZY_BACK=1: no room for the kept pages; the copy-back writes as before");
            return None;
        }
        Some(State {
            slots: slots.cast(),
            cap,
            fresh: 0,
            committed: 0,
            free: Vec::with_capacity(cap),
            seq: 0,
            holds: [(0, 0, 0); HOLDS],
            holds_n: 0,
            holds_over: 0,
            next_hold: 1,
            pins: [(0, 0, 0); PINS],
            pins_n: 0,
            places: [(0, 0); PLACES],
            places_n: 0,
            places_over: false,
        })
    }

    fn slot(&mut self, i: u32) -> &mut Slot {
        // SAFETY: slots below `fresh` are committed and ours.
        unsafe { &mut *self.slots.add(i as usize) }
    }

    /// A slot to keep a page in, if there is room (making room by putting
    /// the oldest back if needed).
    fn take(&mut self) -> Option<u32> {
        if let Some(i) = self.free.pop() {
            return Some(i);
        }
        if self.fresh < self.cap {
            if self.fresh == self.committed {
                let n = CHUNK.min(self.cap - self.committed);
                // SAFETY: inside our reservation.
                let at = unsafe { self.slots.add(self.committed) };
                // SAFETY: committing part of our own reservation.
                if unsafe { VirtualAlloc(at.cast(), n * size_of::<Slot>(), MEM_COMMIT, PAGE_READWRITE) }.is_null() {
                    return None;
                }
                self.committed += n;
            }
            self.fresh += 1;
            return Some(self.fresh as u32 - 1);
        }
        self.make_room();
        self.free.pop()
    }

    /// Puts the oldest sixty-fourth of the kept pages back.
    fn make_room(&mut self) {
        let now = self.seq;
        let mut oldest = u64::MAX;
        for i in 0..self.fresh as u32 {
            let s = self.slot(i);
            if s.page != 0 && s.seq < oldest && s.seq != now {
                oldest = s.seq;
            }
        }
        if oldest == u64::MAX {
            return;
        }
        let line = oldest + ((now - oldest) / 64).max(1);
        for i in 0..self.fresh as u32 {
            let (page, seq) = {
                let s = self.slot(i);
                (s.page, s.seq)
            };
            // Not the pages of the copy-back being kept now.
            if page != 0 && seq < line && seq != now {
                self.put_back(page, Why::Cap);
            }
        }
    }

    /// Takes page `page` out of the tables; its slot is free again.
    fn remove(&mut self, page: usize) -> Option<u32> {
        let g = gib(page)?;
        let i = page % GIB_PAGES;
        let slot = g.slots[i].swap(0, Relaxed);
        g.bits[i / 64].fetch_and(!(1u64 << (i % 64)), Release);
        if slot == 0 {
            return None;
        }
        LIVE.fetch_sub(1, Relaxed);
        let slot = slot - 1;
        self.slot(slot).page = 0;
        self.free.push(slot);
        Some(slot)
    }

    fn slot_of(&mut self, page: usize) -> Option<u32> {
        let g = gib(page)?;
        let slot = g.slots[page % GIB_PAGES].load(Relaxed);
        (slot != 0).then(|| slot - 1)
    }

    /// Puts pending page `page` back into title memory: its protection
    /// back, then the bytes copy-backs put in. Whether title memory now
    /// holds them (false: the page was no longer ours, and was let go).
    fn put_back(&mut self, page: usize, why: Why) -> bool {
        let Some(slot) = self.slot_of(page) else {
            return false;
        };
        let at = page * PAGE;
        let protect = self.slot(slot).protect;
        let mut prev = 0;
        // SAFETY: a page we protected; only its protection changes.
        let ok = unsafe { VirtualProtect(at as *const _, PAGE, protect, &mut prev) } != 0;
        note_in(&RECENT, &RECENT_AT, page);
        if !ok {
            // Not committed any more: freed behind our back.
            self.remove(page);
            count(C::Stale, 1);
            return false;
        }
        if prev != PAGE_NOACCESS {
            // Not ours any more (freed and made again, or protected by
            // someone else): left as it was found.
            let mut ignored = 0;
            // SAFETY: as above.
            unsafe { VirtualProtect(at as *const _, PAGE, prev, &mut ignored) };
            self.remove(page);
            count(C::Stale, 1);
            return false;
        }
        let s = self.slot(slot);
        let mut written = 0u64;
        for_runs(&s.mask, |from, to| {
            // SAFETY: the page is committed and read-write again; the slot
            // is our own memory.
            unsafe { std::ptr::copy_nonoverlapping(s.bytes.as_ptr().add(from), (at + from) as *mut u8, to - from) };
            written += (to - from) as u64;
        });
        self.remove(page);
        count(why.counted(), 1);
        count(C::Written, written);
        true
    }

    /// Lets page `page` go without putting its bytes back, its protection
    /// given back if `restore` (memory that stays, about to be reused).
    fn let_go(&mut self, page: usize, restore: bool) {
        let Some(slot) = self.slot_of(page) else {
            return;
        };
        if restore {
            let protect = self.slot(slot).protect;
            let mut prev = 0;
            let at = page * PAGE;
            // SAFETY: a page we protected; only its protection changes.
            if unsafe { VirtualProtect(at as *const _, PAGE, protect, &mut prev) } != 0 && prev != PAGE_NOACCESS {
                let mut ignored = 0;
                // SAFETY: as above.
                unsafe { VirtualProtect(at as *const _, PAGE, prev, &mut ignored) };
            }
        }
        self.remove(page);
        count(C::Gone, 1);
    }

    fn pins_live(&mut self) {
        let now = now_ms();
        let mut k = 0;
        for i in 0..self.pins_n {
            if self.pins[i].2 > now {
                self.pins[k] = self.pins[i];
                k += 1;
            }
        }
        self.pins_n = k;
    }

    /// Whether `[at, end)` lies under a kernel access, or a heap on title
    /// memory.
    fn held(&mut self, at: usize, end: usize) -> bool {
        if self.holds_over > 0 {
            return true;
        }
        self.pins_live();
        self.holds[..self.holds_n].iter().any(|&(a, e, _)| a < end && at < e)
            || self.pins[..self.pins_n].iter().any(|&(a, e, _)| a < end && at < e)
    }

    fn placed(&self, at: usize, end: usize) -> bool {
        self.places_over || self.places[..self.places_n].iter().any(|&(a, e)| a < end && at < e)
    }
}

/// Calls `f(from, to)` for each run of set bits of a page's mask.
fn for_runs(mask: &[u64; WORDS], mut f: impl FnMut(usize, usize)) {
    let bit = |i: usize| mask[i / 64] >> (i % 64) & 1 != 0;
    let mut i = 0;
    while i < PAGE {
        if mask[i / 64] == 0 && i % 64 == 0 {
            i += 64;
            continue;
        }
        if !bit(i) {
            i += 1;
            continue;
        }
        let from = i;
        while i < PAGE && bit(i) {
            i += 1;
        }
        f(from, i);
    }
}

/// Sets bits `[from, to)` of a page's mask.
fn set_bits(mask: &mut [u64; WORDS], from: usize, to: usize) {
    let mut i = from;
    while i < to {
        let lo = i % 64;
        let n = (64 - lo).min(to - i);
        let bits = if n == 64 { u64::MAX } else { ((1u64 << n) - 1) << lo };
        mask[i / 64] |= bits;
        i += n;
    }
}

/// What [`defer`] did with a copy-back's stretch.
pub(crate) struct Deferred {
    /// Bytes kept here instead of written.
    pub(crate) kept: usize,
    /// The runs (offsets into the stretch, lengths) left to the caller to
    /// write as before.
    pub(crate) left: Vec<(usize, usize)>,
}

/// A copy-back is about to write title memory `[at, at + bytes.len())`:
/// `bytes` is the stretch as it should now be, and `runs` the runs of it
/// (offsets, lengths) to write (`None`: all of it). `bounds` is the title
/// buffer of the copy that wrote it. Keeps what it can of it here (whole
/// pages of private memory inside `bounds`) and gives back what the
/// caller must still write.
pub(crate) fn defer(at: usize, bytes: &[u8], runs: Option<&[(usize, usize)]>, bounds: (usize, usize)) -> Deferred {
    let n = bytes.len();
    let whole = [(0, n)];
    let runs = runs.unwrap_or(&whole);
    let all = || Deferred {
        kept: 0,
        left: runs.to_vec(),
    };
    if n == 0 || runs.is_empty() {
        return Deferred { kept: 0, left: Vec::new() };
    }
    let end = at + n;
    let (first, last) = (at / PAGE, end.div_ceil(PAGE));
    let pages = last - first;
    // Whole pages of the title buffer only; nothing of view memory.
    if crate::inplace::cover(at, n) != crate::inplace::Cover::None {
        count(C::RefuseWhere, pages as u64);
        return all();
    }
    // Room for the answer, before the lock: nothing is allocated under it.
    let mut taken = vec![false; pages];
    let mut eligible = vec![false; pages];
    for (k, page) in (first..last).enumerate() {
        let (a, e) = (page * PAGE, (page + 1) * PAGE);
        if a < bounds.0 || e > bounds.1 {
            count(C::RefuseWhere, 1);
        } else if noted_in(&HOT, &HOT_AT, page, HOT_MS) {
            count(C::RefuseHot, 1);
        } else {
            eligible[k] = true;
        }
    }
    if !eligible.contains(&true) {
        return all();
    }
    crate::watch::install_handler();
    {
        let mut locked = lock();
        if locked.is_none() {
            *locked = State::new();
        }
        let Some(state) = locked.as_mut() else {
            return all();
        };
        if state.placed(first * PAGE, last * PAGE) {
            count(C::RefuseWhere, pages as u64);
            return all();
        }
        state.seq += 1;
        let seq = state.seq;
        // Puts the stretch's runs into a slot's copy of `page`.
        let apply = |s: &mut Slot, page: usize| {
            let (a, e) = (page * PAGE, (page + 1) * PAGE);
            for &(off, len) in runs {
                let (from, to) = ((at + off).max(a), (at + off + len).min(e));
                if from >= to {
                    continue;
                }
                s.bytes[from - a..to - a].copy_from_slice(&bytes[from - at..to - at]);
                set_bits(&mut s.mask, from - a, to - a);
            }
            s.seq = seq;
        };
        // Pages to protect: runs of neighbours taken read-only and kept.
        let mut k = 0;
        while k < pages {
            let page = first + k;
            if !eligible[k] {
                k += 1;
                continue;
            }
            if state.held(page * PAGE, (page + 1) * PAGE) {
                count(C::RefuseHeld, 1);
                k += 1;
                continue;
            }
            if let Some(slot) = state.slot_of(page) {
                apply(state.slot(slot), page);
                taken[k] = true;
                count(C::Again, 1);
                k += 1;
                continue;
            }
            // A run of new pages: each noted pending first (a fault there
            // from now on waits for this lock, and finds the page put
            // back or never taken), made read-only (no title write gets
            // in), read whole and given the stretch's bytes; then all of
            // them made no-access at once.
            let start = k;
            let mut snapped: usize = 0;
            let mut refused = false;
            while k < pages && eligible[k] && !pending(first + k) && !state.held((first + k) * PAGE, (first + k + 1) * PAGE) {
                let page = first + k;
                let a = page * PAGE;
                let i = page % GIB_PAGES;
                let Some(g) = gib_made(page) else {
                    count(C::RefuseFull, 1);
                    refused = true;
                    break;
                };
                let Some(slot) = state.take() else {
                    count(C::RefuseFull, 1);
                    refused = true;
                    break;
                };
                {
                    let s = state.slot(slot);
                    s.mask = [0; WORDS];
                    s.page = page;
                    s.seq = seq;
                }
                g.slots[i].store(slot + 1, Relaxed);
                g.bits[i / 64].fetch_or(1u64 << (i % 64), Release);
                LIVE.fetch_add(1, Relaxed);
                let mut old = 0;
                // SAFETY: a page of title memory; only its protection changes.
                let ok = unsafe { VirtualProtect(a as *const _, PAGE, PAGE_READONLY, &mut old) } != 0;
                if !ok || old != PAGE_READWRITE {
                    if ok {
                        let mut ignored = 0;
                        // SAFETY: as above, back as it was.
                        unsafe { VirtualProtect(a as *const _, PAGE, old, &mut ignored) };
                    }
                    state.remove(page);
                    count(C::RefuseProtect, 1);
                    refused = true;
                    break;
                }
                let s = state.slot(slot);
                // SAFETY: committed and readable (read-only now), our slot.
                unsafe { std::ptr::copy_nonoverlapping(a as *const u8, s.bytes.as_mut_ptr(), PAGE) };
                s.protect = old;
                s.checked = now_ms();
                apply(s, page);
                snapped += 1;
                k += 1;
            }
            if snapped > 0 {
                let a = (first + start) * PAGE;
                let mut old = 0;
                // SAFETY: pages of title memory we just made read-only.
                if unsafe { VirtualProtect(a as *const _, snapped * PAGE, PAGE_NOACCESS, &mut old) } != 0 {
                    for page in first + start..first + start + snapped {
                        taken[page - first] = true;
                    }
                    count(C::New, snapped as u64);
                } else {
                    // Given back as they were; left to the caller.
                    for page in first + start..first + start + snapped {
                        let mut ignored = 0;
                        // SAFETY: as above.
                        unsafe { VirtualProtect((page * PAGE) as *const _, PAGE, PAGE_READWRITE, &mut ignored) };
                        state.remove(page);
                    }
                    count(C::RefuseProtect, snapped as u64);
                }
            }
            // A page refused is not tried again.
            if refused || k == start {
                k += 1;
            }
        }
    }
    // What the pages taken hold of the runs was kept; the rest is the
    // caller's to write.
    let mut left = Vec::new();
    let mut kept = 0usize;
    for &(off, len) in runs {
        let (mut from, to) = (at + off, at + off + len);
        while from < to {
            let page = from / PAGE;
            let stop = ((page + 1) * PAGE).min(to);
            if taken[page - first] {
                kept += stop - from;
            } else if let Some(last) = left.last_mut().filter(|r: &&mut (usize, usize)| r.0 + r.1 == from - at) {
                last.1 += stop - from;
            } else {
                left.push((from - at, stop - from));
            }
            from = stop;
        }
    }
    count(C::Saved, kept as u64);
    Deferred { kept, left }
}

/// Puts the pending pages of `[at, at + len)` back into title memory.
fn put_back_range(at: usize, len: usize, why: Why) {
    if LIVE.load(Relaxed) == 0 || len == 0 {
        return;
    }
    let mut locked = lock();
    let Some(state) = locked.as_mut() else {
        return;
    };
    each_pending(at / PAGE, at.saturating_add(len).div_ceil(PAGE), |page| {
        state.put_back(page, why);
        true
    });
}

/// Lets the pending pages of `[at, at + len)` go unwritten (`restore`:
/// their protection given back).
fn let_go_range(at: usize, len: usize, restore: bool) {
    if LIVE.load(Relaxed) == 0 || len == 0 {
        return;
    }
    let mut locked = lock();
    let Some(state) = locked.as_mut() else {
        return;
    };
    each_pending(at / PAGE, at.saturating_add(len).div_ceil(PAGE), |page| {
        state.let_go(page, restore);
        true
    });
}

/// A kernel write into title memory `[at, end)` is under way: pending
/// pages there are put back first, and none is made there until
/// [`unhold`] with what this returns (0: nothing to give back).
pub(crate) fn hold(at: usize, end: usize) -> u64 {
    if end <= at {
        return 0;
    }
    if !on() {
        put_back_range(at, end - at, Why::Kernel);
        return 0;
    }
    let mut locked = lock();
    if locked.is_none() {
        *locked = State::new();
    }
    let Some(state) = locked.as_mut() else {
        return 0;
    };
    let id = state.next_hold;
    state.next_hold += 1;
    let id = if state.holds_n < HOLDS {
        state.holds[state.holds_n] = (at, end, id);
        state.holds_n += 1;
        id
    } else {
        state.holds_over += 1;
        u64::MAX
    };
    if LIVE.load(Relaxed) != 0 {
        each_pending(at / PAGE, end.div_ceil(PAGE), |page| {
            state.put_back(page, Why::Kernel);
            true
        });
    }
    id
}

/// The kernel access [`hold`] was told of is done.
pub(crate) fn unhold(id: u64) {
    if id == 0 {
        return;
    }
    let mut locked = lock();
    let Some(state) = locked.as_mut() else {
        return;
    };
    if id == u64::MAX {
        state.holds_over = state.holds_over.saturating_sub(1);
        return;
    }
    if let Some(i) = state.holds[..state.holds_n].iter().position(|h| h.2 == id) {
        state.holds_n -= 1;
        state.holds[i] = state.holds[state.holds_n];
    }
}

/// A kernel access to `[at, end)` may still run for `ms`: pending pages
/// there are put back now (as `why`), and none is made there meanwhile.
fn pin(at: usize, end: usize, ms: u64, why: Why) {
    if end <= at {
        return;
    }
    if !on() {
        put_back_range(at, end - at, why);
        return;
    }
    let mut locked = lock();
    if locked.is_none() {
        *locked = State::new();
    }
    let Some(state) = locked.as_mut() else {
        return;
    };
    state.pins_live();
    if state.pins_n < PINS {
        state.pins[state.pins_n] = (at, end, now_ms() + ms);
        state.pins_n += 1;
    } else {
        // No room to remember it: the oldest pin is stretched to cover it.
        let p = &mut state.pins[0];
        *p = (p.0.min(at), p.1.max(end), p.2.max(now_ms() + ms));
    }
    if LIVE.load(Relaxed) != 0 {
        each_pending(at / PAGE, end.div_ceil(PAGE), |page| {
            state.put_back(page, why);
            true
        });
    }
}

/// An asynchronous kernel write into `[at, end)` may land for `ms` more.
pub(crate) fn pin_write(at: usize, end: usize, ms: u64) {
    pin(at, end, ms, Why::Kernel);
}

/// The runtime is about to have the kernel read title memory
/// `[at, at + len)` (a file written from a title buffer): pending pages
/// there are put back first, and none is made there for a while (the
/// read may be overlapped).
pub fn kernel_will_read(at: usize, len: usize) {
    pin(at, at.saturating_add(len), PIN_MS, Why::Read);
}

/// Title memory `[at, at + len)` is about to be committed at its address,
/// or opened as a heap the card reads directly: pending pages there are put
/// back first.
pub(crate) fn changing(at: usize, len: usize) {
    put_back_range(at, len, Why::Remap);
}

/// The card reads (and writes) title memory `[at, at + len)` itself from
/// now on (a heap opened on it): pending pages there are put back, and
/// none is ever made there again.
pub(crate) fn in_place(at: usize, len: usize) {
    put_back_range(at, len, Why::Remap);
    if !on() {
        return;
    }
    let mut locked = lock();
    if locked.is_none() {
        *locked = State::new();
    }
    let Some(state) = locked.as_mut() else {
        return;
    };
    if state.places_n < PLACES {
        state.places[state.places_n] = (at, at.saturating_add(len));
        state.places_n += 1;
    } else {
        state.places_over = true;
    }
}

/// The allocation that begins at `base` is opened as a heap the card
/// reads directly: as [`in_place`] for all of it.
pub(crate) fn in_place_allocation(base: usize) {
    let (start, len) = allocation(base);
    if len == 0 {
        // Not known: nothing is made pending anywhere from now on.
        put_back_range(base, 64 << 30, Why::Remap);
        if on() {
            let mut locked = lock();
            if let Some(state) = locked.as_mut() {
                state.places_over = true;
            }
        }
        return;
    }
    in_place(start, len);
}

/// The allocation at `base`: its start and bytes (0 if not one).
fn allocation(base: usize) -> (usize, usize) {
    let mut at = base;
    loop {
        // SAFETY: plain data filled in by the query.
        let mut info: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: a valid out pointer of the right size.
        if unsafe { VirtualQuery(at as *const _, &mut info, size_of_val(&info)) } == 0 || info.AllocationBase as usize != base {
            return (base, at - base);
        }
        at = info.BaseAddress as usize + info.RegionSize;
    }
}

/// Title memory `[at, at + len)` changed meaning (freed, remapped, newly
/// reserved): pending pages there are let go, unwritten (`restore`: their
/// protection given back, for memory that stays mapped).
pub(crate) fn gone(at: usize, len: usize, restore: bool) {
    let_go_range(at, len, restore);
}

/// The allocation at `base` is about to be released: its pending pages are
/// let go. Only pending pages are looked at (their protection is ours, so
/// asking Windows about them is quick).
pub(crate) fn releasing(base: usize) {
    if LIVE.load(Relaxed) == 0 {
        return;
    }
    let mut locked = lock();
    let Some(state) = locked.as_mut() else {
        return;
    };
    // An allocation the title releases this way is at most a few GiB.
    let mut until = 0usize;
    let mut same = false;
    each_pending(base / PAGE, base.saturating_add(16 << 30).div_ceil(PAGE), |page| {
        let at = page * PAGE;
        if at >= until {
            // SAFETY: plain data filled in by the query.
            let mut info: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
            // SAFETY: a valid out pointer of the right size.
            if unsafe { VirtualQuery(at as *const _, &mut info, size_of_val(&info)) } == 0 {
                same = false;
                until = at + PAGE;
            } else {
                same = info.AllocationBase as usize == base;
                until = info.BaseAddress as usize + info.RegionSize;
            }
        }
        if same {
            state.let_go(page, false);
        }
        true
    });
}

/// The runtime is about to commit title memory `[at, at + len)` at its
/// address (a commit changes the protection of pages already committed):
/// copy-back pages left pending there go into it first.
pub fn title_memory_commit(at: usize, len: usize) {
    changing(at, len);
}

/// The runtime reserved new title memory `[at, at + len)`: anything kept
/// for those addresses is of memory freed behind our back, and goes.
pub fn title_memory_reserved(at: usize, len: usize) {
    gone(at, len, false);
}

/// The runtime is about to free title memory `[at, at + len)` (it stays
/// mapped, for reuse), or with `len` 0 to release the allocation at `at`:
/// copy-back pages left pending there are let go unwritten.
pub fn title_memory_release(at: usize, len: usize) {
    if len == 0 {
        releasing(at);
    } else {
        gone(at, len, true);
    }
}

/// From the fault handlers, for an access violation at `at` (`write`: a
/// write): whether it was a pending page's, now put back (or one another
/// thread just put back), so the access can go on.
pub(crate) fn fault(at: usize, write: bool) -> bool {
    if HOLDING.get() {
        return false;
    }
    let page = at / PAGE;
    // Pending, or put back by another thread between the fault and here.
    if !(LIVE.load(Relaxed) != 0 && pending(page)) && !noted_in(&RECENT, &RECENT_AT, page, 10_000) {
        return false;
    }
    let started = std::time::Instant::now();
    // Answered under the lock: a copy-back may make the page pending again
    // the moment it is free, and the question asked after that would say
    // no to an access that only needs to be tried again.
    let go_on = {
        let mut locked = lock();
        match locked.as_mut() {
            Some(state) if pending(page) => {
                let back = state.put_back(page, Why::Fault);
                note_in(&HOT, &HOT_AT, page);
                // One found no longer ours was left as it was: the access
                // goes on only if it can.
                back || allows(at, write)
            }
            _ => allows(at, write),
        }
    };
    count(C::HandlerNs, started.elapsed().as_nanos() as u64);
    go_on
}

/// Whether `at` is committed and its protection lets the access (a write
/// with `write`) go on now.
fn allows(at: usize, write: bool) -> bool {
    use windows_sys::Win32::System::Memory::{
        PAGE_EXECUTE_READ, PAGE_EXECUTE_READWRITE, PAGE_EXECUTE_WRITECOPY, PAGE_GUARD, PAGE_WRITECOPY,
    };
    // SAFETY: plain data filled in by the query.
    let mut info: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: a valid out pointer of the right size.
    let n = unsafe { VirtualQuery(at as *const _, &mut info, size_of_val(&info)) };
    if n == 0 || info.State != MEM_COMMIT || info.Protect & PAGE_GUARD != 0 {
        return false;
    }
    let p = info.Protect & 0xff;
    let writable = matches!(p, PAGE_READWRITE | PAGE_WRITECOPY | PAGE_EXECUTE_READWRITE | PAGE_EXECUTE_WRITECOPY);
    let readable = writable || matches!(p, PAGE_READONLY | PAGE_EXECUTE_READ);
    if write { writable } else { readable }
}

/// Reads title memory `[src, src + len)` into `dst` where pages are
/// pending: those from what is kept, the rest with `raw`. Whether all of
/// it was read (as `raw` answers).
pub(crate) fn read_over(src: usize, dst: *mut u8, len: usize, raw: impl Fn(*const u8, *mut u8, usize) -> bool) -> bool {
    let (first, last) = (src / PAGE, src.saturating_add(len).div_ceil(PAGE));
    let mut seen = Vec::new();
    each_pending(first, last, |page| {
        seen.push(page);
        true
    });
    let end = src + len;
    let piece = |page: usize| (src.max(page * PAGE), end.min((page + 1) * PAGE));
    // What is not pending: read as ever, outside the lock.
    let mut whole = true;
    let mut at = src;
    for &page in &seen {
        let (from, _) = piece(page);
        if from > at {
            whole &= raw(at as *const u8, dst.wrapping_add(at - src), from - at);
        }
        at = piece(page).1;
    }
    if at < end {
        whole &= raw(at as *const u8, dst.wrapping_add(at - src), end - at);
    }
    // Pending pages from here; one put back since is read as ever. (Room
    // made before the lock: nothing is allocated under it.)
    let mut later = Vec::with_capacity(seen.len());
    {
        let mut locked = lock();
        if let Some(state) = locked.as_mut() {
            let now = now_ms();
            for &page in &seen {
                let Some(slot) = state.slot_of(page).filter(|_| pending(page)) else {
                    later.push(page);
                    continue;
                };
                if now.saturating_sub(state.slot(slot).checked) > CHECK_MS && !still_ours(state, page, now) {
                    later.push(page);
                    continue;
                }
                let (from, to) = piece(page);
                let s = state.slot(slot);
                // SAFETY: `dst` holds `len` bytes; the slot is our memory.
                unsafe {
                    std::ptr::copy_nonoverlapping(s.bytes.as_ptr().add(from - page * PAGE), dst.add(from - src), to - from)
                };
                count(C::Served, 1);
            }
        } else {
            later.extend_from_slice(&seen);
        }
    }
    for page in later {
        let (from, to) = piece(page);
        whole &= raw(from as *const u8, dst.wrapping_add(from - src), to - from);
    }
    whole
}

/// Whether pending page `page` is still no-access private memory (ours);
/// if not it is let go (it was freed and made again behind our back). The
/// pending neighbours the answer covers are noted checked too.
fn still_ours(state: &mut State, page: usize, now: u64) -> bool {
    let at = page * PAGE;
    // SAFETY: plain data filled in by the query.
    let mut info: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: a valid out pointer of the right size. The page is no-access,
    // so the region it answers is our island of pages: a quick answer.
    let n = unsafe { VirtualQuery(at as *const _, &mut info, size_of_val(&info)) };
    if n == 0 || info.State != MEM_COMMIT || info.Type != MEM_PRIVATE || info.Protect != PAGE_NOACCESS {
        state.remove(page);
        count(C::Stale, 1);
        return false;
    }
    let end = (info.BaseAddress as usize + info.RegionSize).div_ceil(PAGE).min(page + 256);
    for p in page..end {
        if let Some(slot) = state.slot_of(p) {
            state.slot(slot).checked = now;
        }
    }
    true
}

/// Pending pages now and their bytes kept (with their masks).
pub(crate) fn held_now() -> (u64, u64) {
    let n = LIVE.load(Relaxed);
    (n, n * size_of::<Slot>() as u64)
}

/// With the frame line: what was done per frame over `frames`, and since
/// the start.
pub(crate) fn line(frames: f64) {
    if !on() && TOTAL[C::New as usize].load(Relaxed) == 0 {
        return;
    }
    let c: [u64; COUNTS] = std::array::from_fn(|i| COUNT[i].swap(0, Relaxed));
    let t: [u64; COUNTS] = std::array::from_fn(|i| TOTAL[i].load(Relaxed));
    let per = |c: C| c as usize;
    let (held, bytes) = held_now();
    let f = |n: u64| n as f64 / frames;
    tracing::info!(
        "copy-back on demand per frame (VICTORY_LAZY_BACK=1): {:.0} pages left pending, {:.0} copy-backs into pages already \
         pending, {:.1} MB not written into title memory; put back {:.1} on a processor's touch ({:.2} ms in the handler), {:.1} \
         for a kernel or eager write, {:.1} for a kernel read, {:.1} for a commit or a heap on title memory, {:.1} for room, \
         {:.1} MB written so; {:.1} let go with their memory, {:.1} found no longer ours; {:.0} pages our reads took from here; \
         left to the copy-back: {:.0} written by the title lately, {:.0} not whole in a title buffer of private memory, {:.0} \
         under a kernel access, {:.0} not read-write, {:.0} for want of room; held now {held} pages, {:.1} MB of {} MB; since \
         the start {:.0} MB not written, {} pages put back on a touch",
        f(c[per(C::New)]),
        f(c[per(C::Again)]),
        f(c[per(C::Saved)]) / 1e6,
        f(c[per(C::Fault)]),
        f(c[per(C::HandlerNs)]) / 1e6,
        f(c[per(C::Kernel)]),
        f(c[per(C::Read)]),
        f(c[per(C::Remap)]),
        f(c[per(C::Cap)]),
        f(c[per(C::Written)]) / 1e6,
        f(c[per(C::Gone)]),
        f(c[per(C::Stale)]),
        f(c[per(C::Served)]),
        f(c[per(C::RefuseHot)]),
        f(c[per(C::RefuseWhere)]),
        f(c[per(C::RefuseHeld)]),
        f(c[per(C::RefuseProtect)]),
        f(c[per(C::RefuseFull)]),
        bytes as f64 / 1e6,
        (cap_pages() * PAGE) >> 20,
        t[per(C::Saved)] as f64 / 1e6,
        t[per(C::Fault)],
    );
}

/// For tests: the switch on for this thread alone.
#[cfg(test)]
pub(crate) fn test_on() {
    TEST_ON.set(true);
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::System::Memory::{MEM_RELEASE, VirtualFree};

    /// Memory of the test's own, `pages` committed read-write pages.
    struct Own {
        base: usize,
        pages: usize,
    }

    impl Own {
        fn new(pages: usize) -> Own {
            // SAFETY: memory of the test's own.
            let base = unsafe { VirtualAlloc(std::ptr::null(), pages * PAGE, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE) } as usize;
            assert_ne!(base, 0);
            Own { base, pages }
        }

        fn bounds(&self) -> (usize, usize) {
            (self.base, self.base + self.pages * PAGE)
        }
    }

    impl Drop for Own {
        fn drop(&mut self) {
            gone(self.base, self.pages * PAGE, false);
            // Its addresses may be another test's next: no pin or touch of
            // this one's stays on them.
            let end = self.base + self.pages * PAGE;
            {
                let mut locked = lock();
                if let Some(s) = locked.as_mut() {
                    s.pins_live();
                    let mut k = 0;
                    for i in 0..s.pins_n {
                        if s.pins[i].1 <= self.base || s.pins[i].0 >= end {
                            s.pins[k] = s.pins[i];
                            k += 1;
                        }
                    }
                    s.pins_n = k;
                }
            }
            for page in self.base / PAGE..end / PAGE {
                HOT[page & 4095].store(0, Relaxed);
            }
            // SAFETY: the test's reservation.
            unsafe { VirtualFree(self.base as *mut _, 0, MEM_RELEASE) };
        }
    }

    fn protection(at: usize) -> u32 {
        // SAFETY: plain data filled in by the query.
        let mut info: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
        // SAFETY: a valid out pointer of the right size.
        assert_ne!(unsafe { VirtualQuery(at as *const _, &mut info, size_of_val(&info)) }, 0);
        info.Protect
    }

    #[test]
    fn mask_runs_are_the_bits_set() {
        let mut mask = [0u64; WORDS];
        set_bits(&mut mask, 3, 9);
        set_bits(&mut mask, 60, 130);
        set_bits(&mut mask, 4000, 4096);
        set_bits(&mut mask, 0, 0);
        let mut runs = Vec::new();
        for_runs(&mask, |a, b| runs.push((a, b)));
        assert_eq!(runs, vec![(3, 9), (60, 130), (4000, 4096)]);
        let mut all = [0u64; WORDS];
        set_bits(&mut all, 0, PAGE);
        let mut runs = Vec::new();
        for_runs(&all, |a, b| runs.push((a, b)));
        assert_eq!(runs, vec![(0, PAGE)]);
    }

    #[test]
    fn pending_pages_are_found_by_range() {
        test_on();
        let own = Own::new(70);
        // Pages 1, 2 and 66 (across a mask word) pending.
        for page in [1usize, 2, 66] {
            let at = own.base + page * PAGE;
            let bytes = vec![7u8; 16];
            let d = defer(at + 100, &bytes, None, own.bounds());
            assert_eq!(d.kept, 16);
            assert!(d.left.is_empty());
        }
        let p0 = own.base / PAGE;
        let mut found = Vec::new();
        each_pending(p0, p0 + 70, |p| {
            found.push(p - p0);
            true
        });
        assert_eq!(found, vec![1, 2, 66]);
        assert!(touches(own.base + 2 * PAGE + 5, 1));
        assert!(!touches(own.base, PAGE));
        assert!(!touches(own.base + 3 * PAGE, 63 * PAGE));
        assert!(touches(own.base, 70 * PAGE));
    }

    #[test]
    fn a_pending_page_is_read_as_the_gpu_left_it_and_a_touch_puts_it_back() {
        test_on();
        let own = Own::new(4);
        let at = own.base + PAGE;
        // SAFETY: the test's memory.
        unsafe { std::slice::from_raw_parts_mut(own.base as *mut u8, 4 * PAGE) }.fill(0x11);
        // The GPU wrote bytes 10..20 and 4000..4100 (across into page 2).
        let mut stretch = vec![0x11u8; 4100];
        stretch[10..20].fill(0xAA);
        stretch[4000..4100].fill(0xBB);
        let d = defer(at, &stretch, Some(&[(10, 10), (4000, 100)]), own.bounds());
        assert_eq!(d.kept, 110);
        assert!(d.left.is_empty());
        assert_eq!(protection(at), PAGE_NOACCESS);
        assert_eq!(protection(at + PAGE), PAGE_NOACCESS);
        // Our reads see what the GPU wrote, and trip nothing.
        let mut read = vec![0u8; 3 * PAGE];
        assert!(crate::host::read_into(own.base as *const u8, &mut read));
        assert!(read[..PAGE + 10].iter().all(|&b| b == 0x11));
        assert!(read[PAGE + 10..PAGE + 20].iter().all(|&b| b == 0xAA));
        assert!(read[PAGE + 4000..PAGE + 4100].iter().all(|&b| b == 0xBB));
        assert_eq!(protection(at), PAGE_NOACCESS, "a read of ours leaves it pending");
        // The processor's touch faults, puts it back and goes on.
        let before = TOTAL[C::Fault as usize].load(Relaxed);
        // SAFETY: the test's memory; the fault is taken by our handler.
        let byte = unsafe { std::ptr::read_volatile((at + 15) as *const u8) };
        assert_eq!(byte, 0xAA);
        assert!(TOTAL[C::Fault as usize].load(Relaxed) > before);
        assert_eq!(protection(at), PAGE_READWRITE);
        // SAFETY: as above, a write into the other page.
        unsafe { std::ptr::write_volatile((at + PAGE + 50) as *mut u8, 0xCC) };
        // SAFETY: the test's memory, all of it read-write again.
        let all = unsafe { std::slice::from_raw_parts(own.base as *const u8, 4 * PAGE) };
        assert!(all[PAGE + 10..PAGE + 20].iter().all(|&b| b == 0xAA));
        assert!(all[2 * PAGE..2 * PAGE + 4].iter().all(|&b| b == 0xBB));
        assert_eq!(all[2 * PAGE + 50], 0xCC);
        assert_eq!(all[2 * PAGE + 4], 0x11, "the title's bytes beside the GPU's stay");
    }

    #[test]
    fn a_kernel_write_puts_pending_pages_back_first() {
        use windows_sys::Win32::System::Diagnostics::Debug::{ReadProcessMemory, WriteProcessMemory};
        use windows_sys::Win32::System::Threading::GetCurrentProcess;
        test_on();
        let own = Own::new(2);
        let mut stretch = vec![0u8; 64];
        stretch[..8].fill(0x5A);
        let d = defer(own.base + 64, &stretch, Some(&[(0, 8)]), own.bounds());
        assert_eq!(d.kept, 8);
        // A kernel read without telling fails on the pending page.
        let mut out = [0u8; 8];
        let mut done = 0;
        // SAFETY: our own process and buffers.
        let ok = unsafe { ReadProcessMemory(GetCurrentProcess(), (own.base + 64) as *const _, out.as_mut_ptr().cast(), 8, &mut done) };
        assert_eq!(ok, 0, "the kernel does not fault, it fails");
        // Told, the kernel finds the GPU's bytes.
        kernel_will_read(own.base, 2 * PAGE);
        // SAFETY: as above.
        let ok = unsafe { ReadProcessMemory(GetCurrentProcess(), (own.base + 64) as *const _, out.as_mut_ptr().cast(), 8, &mut done) };
        assert_ne!(ok, 0);
        assert_eq!(out, [0x5A; 8]);
        // A kernel write the runtime announces (`kernel_will_write`).
        let d = defer(own.base + 64, &stretch, Some(&[(0, 8)]), own.bounds());
        assert_eq!(d.kept, 0, "pinned by the read: written as before");
        let page = own.base + PAGE;
        let mut stretch = vec![0u8; 8];
        stretch.fill(0x3C);
        gone(own.base, 2 * PAGE, true);
        {
            let mut locked = lock();
            if let Some(s) = locked.as_mut() {
                s.pins_n = 0;
            }
        }
        let d = defer(page, &stretch, None, own.bounds());
        assert_eq!(d.kept, 8);
        let writing = crate::watch::writing_held(page, 4);
        assert_eq!(protection(page), PAGE_READWRITE, "put back before the kernel writes");
        let new = [0x77u8; 4];
        // SAFETY: as above.
        let ok = unsafe { WriteProcessMemory(GetCurrentProcess(), page as *const _, new.as_ptr().cast(), 4, &mut done) };
        assert_ne!(ok, 0);
        // While the write is held nothing is made pending there.
        assert_eq!(defer(page, &stretch, None, own.bounds()).kept, 0);
        drop(writing);
        // SAFETY: the test's memory, read-write.
        let back = unsafe { std::slice::from_raw_parts(page as *const u8, 8) };
        assert_eq!(back, &[0x77, 0x77, 0x77, 0x77, 0x3C, 0x3C, 0x3C, 0x3C]);
    }

    /// A title thread reading and writing a page while copy-backs keep
    /// making it pending: every access goes on (none falls through the
    /// handler, in the window where the page is read-only either), and the
    /// last copy-back's bytes are what title memory holds in the end.
    #[test]
    fn a_title_thread_racing_the_copy_backs_is_never_stopped() {
        use std::sync::atomic::AtomicBool;
        test_on();
        let own = Own::new(2);
        let (base, bounds) = (own.base, own.bounds());
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        let touches = std::sync::Arc::new(AtomicU64::new(0));
        let toucher = {
            let (stop, touches) = (stop.clone(), touches.clone());
            std::thread::spawn(move || {
                while !stop.load(Relaxed) {
                    // SAFETY: the test's memory; faults are taken by our
                    // handler.
                    unsafe {
                        let v = std::ptr::read_volatile((base + 8) as *const u32);
                        std::ptr::write_volatile((base + 64) as *mut u32, v.wrapping_add(1));
                    }
                    touches.fetch_add(1, Relaxed);
                }
            })
        };
        let mut last = 0;
        let started = std::time::Instant::now();
        // Until the other thread has run alongside for a while (or long).
        let mut i = 0u32;
        while i < 2000 || (touches.load(Relaxed) < 1000 && started.elapsed().as_secs() < 10) {
            i += 1;
            // Not left alone for being touched: the race is what is tried.
            HOT[(base / PAGE) & 4095].store(0, Relaxed);
            let d = defer(base + 8, &i.to_le_bytes(), None, bounds);
            if d.kept == 4 {
                last = i;
            } else {
                // Left to the caller: written as the copy-back would.
                // SAFETY: the test's memory.
                unsafe { std::ptr::write_volatile((base + 8) as *mut u32, i) };
                last = i;
            }
        }
        stop.store(true, Relaxed);
        toucher.join().unwrap();
        assert!(touches.load(Relaxed) > 0);
        kernel_will_read(base, PAGE);
        // SAFETY: the test's memory, read-write again.
        assert_eq!(unsafe { std::ptr::read_volatile((base + 8) as *const u32) }, last);
    }

    #[test]
    fn pages_outside_the_buffer_and_freed_pages_are_never_written() {
        test_on();
        let own = Own::new(3);
        // A stretch over the buffer's first page edge: only whole pages
        // inside the bounds are kept.
        let bounds = (own.base + 100, own.base + 3 * PAGE);
        let stretch = vec![9u8; PAGE];
        let d = defer(own.base + 200, &stretch, None, bounds);
        assert_eq!(d.kept, 200, "only the part in the second page");
        assert_eq!(d.left, vec![(0, PAGE - 200)]);
        assert_eq!(protection(own.base), PAGE_READWRITE);
        assert_eq!(protection(own.base + PAGE), PAGE_NOACCESS);
        // Freed behind our back and made again: never written over.
        // SAFETY: decommitting and committing the test's own page.
        unsafe {
            VirtualFree((own.base + PAGE) as *mut _, PAGE, windows_sys::Win32::System::Memory::MEM_DECOMMIT);
            VirtualAlloc((own.base + PAGE) as *const _, PAGE, MEM_COMMIT, PAGE_READWRITE);
        }
        put_back_range(own.base, 3 * PAGE, Why::Cap);
        // SAFETY: committed again, read-write, zeroed.
        assert!(unsafe { std::slice::from_raw_parts((own.base + PAGE) as *const u8, PAGE) }.iter().all(|&b| b == 0));
        assert!(!touches(own.base, 3 * PAGE));
    }
}
