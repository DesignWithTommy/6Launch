//! VICTORY_READBACK_TRACE=1 (diag): copied-back pages get a one-shot guard; first touches are counted by code address.

use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

const PAGE: usize = 4096;
// past this many armed pages something else took the guards off: forget them
const MAX_ARMED: usize = 1 << 20;

pub(crate) fn on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        cfg!(feature = "diagnostics")
            && std::env::var_os("VICTORY_READBACK_TRACE").is_some_and(|v| v == "1")
    })
}

#[derive(Default)]
struct Site {
    hits: u64,
    // a data address it touched, to match against buffers
    sample: usize,
}

struct State {
    armed: BTreeSet<usize>,
    // (rip, write) -> hits
    sites: HashMap<(usize, bool), Site>,
    armed_total: u64,
    hits_total: u64,
    logged: Instant,
}

fn state() -> std::sync::MutexGuard<'static, State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE
        .get_or_init(|| {
            Mutex::new(State {
                armed: BTreeSet::new(),
                sites: HashMap::new(),
                armed_total: 0,
                hits_total: 0,
                logged: Instant::now(),
            })
        })
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

fn pages(at: usize, len: usize) -> std::ops::Range<usize> {
    at / PAGE..at.saturating_add(len).div_ceil(PAGE)
}

// current protection of a committed page, `None` if not committed
fn protection(page: usize) -> Option<u32> {
    use windows_sys::Win32::System::Memory::{MEM_COMMIT, MEMORY_BASIC_INFORMATION, VirtualQuery};
    // SAFETY: plain data filled in by the query.
    let mut info: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: a valid out pointer of the right size; the page is not read.
    let n = unsafe { VirtualQuery((page * PAGE) as *const _, &mut info, size_of_val(&info)) };
    (n != 0 && info.State == MEM_COMMIT).then_some(info.Protect)
}

fn protect(page: usize, value: u32) -> bool {
    use windows_sys::Win32::System::Memory::VirtualProtect;
    let mut old = 0;
    // SAFETY: changes one committed page's protection; nothing is read.
    unsafe { VirtualProtect((page * PAGE) as *const _, PAGE, value, &mut old) != 0 }
}

fn arm_page(page: usize) -> bool {
    use windows_sys::Win32::System::Memory::{PAGE_GUARD, PAGE_NOACCESS};
    match protection(page) {
        Some(p) if p & PAGE_GUARD != 0 => true,
        Some(p) if p != PAGE_NOACCESS && p != 0 => protect(page, p | PAGE_GUARD),
        _ => false,
    }
}

fn disarm_page(page: usize) {
    use windows_sys::Win32::System::Memory::PAGE_GUARD;
    if let Some(p) = protection(page)
        && p & PAGE_GUARD != 0
    {
        protect(page, p & !PAGE_GUARD);
    }
}

// copy-back just wrote `[at, at + len)` of title memory
pub(crate) fn arm(at: usize, len: usize) {
    if !on() || len == 0 {
        return;
    }
    install();
    let mut s = state();
    if s.armed.len() > MAX_ARMED {
        s.armed.clear();
    }
    for page in pages(at, len) {
        if arm_page(page) {
            s.armed.insert(page);
            s.armed_total += 1;
        }
    }
    report(&mut s);
}

// before our own kernel call on `[at, at + len)` (it fails on guards): the armed pages, unguarded
pub(crate) fn hold(at: usize, len: usize) -> Vec<usize> {
    if !on() || len == 0 {
        return Vec::new();
    }
    let mut s = state();
    let held: Vec<usize> = s.armed.range(pages(at, len)).copied().collect();
    for &page in &held {
        s.armed.remove(&page);
        disarm_page(page);
    }
    held
}

// after our own read: guards back on what `hold` took
pub(crate) fn rearm(held: Vec<usize>) {
    if held.is_empty() {
        return;
    }
    let mut s = state();
    for page in held {
        if arm_page(page) {
            s.armed.insert(page);
        }
    }
}

// a guard fault on an armed page: counted, and the access goes on (the
// system took the guard off already)
fn hit(page: usize, rip: usize, write: bool, address: usize) -> bool {
    let mut s = state();
    if !s.armed.remove(&page) {
        return false;
    }
    s.hits_total += 1;
    let site = s.sites.entry((rip, write)).or_default();
    site.hits += 1;
    if site.sample == 0 {
        site.sample = address;
    }
    true
}

fn install() {
    use windows_sys::Win32::Foundation::{EXCEPTION_GUARD_PAGE, NTSTATUS};
    use windows_sys::Win32::System::Diagnostics::Debug::{
        AddVectoredExceptionHandler, EXCEPTION_CONTINUE_EXECUTION, EXCEPTION_CONTINUE_SEARCH,
        EXCEPTION_POINTERS,
    };
    unsafe extern "system" fn on_guard(info: *mut EXCEPTION_POINTERS) -> i32 {
        // SAFETY: the system passes the exception being dispatched.
        let (r, context) = unsafe { (&*(*info).ExceptionRecord, &*(*info).ContextRecord) };
        if r.ExceptionCode != EXCEPTION_GUARD_PAGE as NTSTATUS || r.NumberParameters < 2 {
            return EXCEPTION_CONTINUE_SEARCH;
        }
        let address = r.ExceptionInformation[1];
        // 0 read, 1 write, 8 execute
        let write = r.ExceptionInformation[0] == 1;
        if hit(address / PAGE, context.Rip as usize, write, address) {
            EXCEPTION_CONTINUE_EXECUTION
        } else {
            EXCEPTION_CONTINUE_SEARCH
        }
    }
    static DONE: AtomicBool = AtomicBool::new(false);
    if DONE.swap(true, Ordering::AcqRel) {
        return;
    }
    // SAFETY: the handler touches only our state and page protections.
    unsafe {
        AddVectoredExceptionHandler(1, Some(on_guard));
    }
}

fn locate(address: usize) -> String {
    use windows_sys::Win32::System::LibraryLoader::{
        GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
        GetModuleFileNameW, GetModuleHandleExW,
    };
    let mut module = std::ptr::null_mut();
    // SAFETY: looks up the module holding `address`; no reference taken.
    let found = unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            address as *const u16,
            &mut module,
        )
    };
    if found == 0 {
        return format!("{address:#x}");
    }
    let mut path = [0u16; 260];
    // SAFETY: a loaded module and the buffer's length.
    let len = unsafe { GetModuleFileNameW(module, path.as_mut_ptr(), path.len() as u32) };
    let path = String::from_utf16_lossy(&path[..len as usize]);
    let name = path.rsplit(['\\', '/']).next().unwrap_or(&path).to_owned();
    format!("{name}+{:#x}", address - module as usize)
}

fn report(s: &mut State) {
    if s.logged.elapsed().as_secs() < 10 {
        return;
    }
    s.logged = Instant::now();
    let mut top: Vec<_> = s.sites.iter().map(|(&(rip, write), site)| (site.hits, rip, write, site.sample)).collect();
    top.sort_unstable_by(|a, b| b.0.cmp(&a.0));
    let line = top
        .iter()
        .take(24)
        .map(|&(hits, rip, write, sample)| {
            format!("{} {} x{hits} @{sample:#x}", locate(rip), if write { "w" } else { "r" })
        })
        .collect::<Vec<_>>()
        .join("; ");
    tracing::info!(
        armed = s.armed_total,
        hits = s.hits_total,
        sites = s.sites.len(),
        "readback trace (first touch of copied-back pages, since start): {line}"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_cover_the_range() {
        assert_eq!(pages(0, 1), 0..1);
        assert_eq!(pages(PAGE - 1, 2), 0..2);
        assert_eq!(pages(PAGE, PAGE), 1..2);
        assert_eq!(pages(3 * PAGE + 5, 0), 3..4);
    }

    #[test]
    fn an_armed_page_faults_once_and_is_counted() {
        use windows_sys::Win32::System::Memory::{
            MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE, VirtualAlloc, VirtualFree,
        };
        // SAFETY: a fresh allocation, freed at the end.
        let p = unsafe { VirtualAlloc(std::ptr::null(), 2 * PAGE, MEM_RESERVE | MEM_COMMIT, PAGE_READWRITE) } as usize;
        assert_ne!(p, 0);
        install();
        assert!(arm_page(p / PAGE + 1));
        state().armed.insert(p / PAGE + 1);
        let before = state().hits_total;
        // SAFETY: committed pages of ours; the guard fault is taken by the handler.
        let v = unsafe { std::ptr::read_volatile((p + PAGE + 8) as *const u8) };
        assert_eq!(v, 0);
        // SAFETY: as above, the guard is gone now.
        let _ = unsafe { std::ptr::read_volatile((p + PAGE + 16) as *const u8) };
        assert_eq!(state().hits_total, before + 1);
        assert!(!state().armed.contains(&(p / PAGE + 1)));
        // held and put back: the guard comes off and on again
        assert!(arm_page(p / PAGE));
        state().armed.insert(p / PAGE);
        let held = {
            let mut s = state();
            let held: Vec<usize> = s.armed.range(pages(p, 1)).copied().collect();
            for &page in &held {
                s.armed.remove(&page);
                disarm_page(page);
            }
            held
        };
        assert_eq!(held, vec![p / PAGE]);
        assert_eq!(protection(p / PAGE), Some(PAGE_READWRITE));
        rearm(held);
        assert!(state().armed.contains(&(p / PAGE)));
        // SAFETY: our allocation.
        unsafe { VirtualFree(p as *mut _, 0, MEM_RELEASE) };
    }
}
