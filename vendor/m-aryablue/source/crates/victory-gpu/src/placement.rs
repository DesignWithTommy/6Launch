//! Where our process and helper threads run; this process only, never
//! system settings.
//!
//! - `VICTORY_PRIORITY=above`: the process's priority class goes to above
//!   normal at the start.
//! - `VICTORY_IDEAL=1`: our helper threads (submit, relay, late wait, wide,
//!   land, texture worker, heap ahead) get soft ideal processors counted
//!   down from the last one; `VICTORY_IDEAL=15,13,11` gives the order.
//! - `VICTORY_THREAD_MAP=1`: every 60 s the busy threads of the process are
//!   logged (warn) with their name, processor time, affinity, ideal
//!   processor and priority.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

/// At the start of a run: the priority class and the thread map, as asked.
pub fn start() {
    if std::env::var("VICTORY_PRIORITY").is_ok_and(|v| v.trim().eq_ignore_ascii_case("above")) {
        use windows_sys::Win32::System::Threading::{ABOVE_NORMAL_PRIORITY_CLASS, GetCurrentProcess, SetPriorityClass};
        // SAFETY: our own process's pseudo handle.
        let ok = unsafe { SetPriorityClass(GetCurrentProcess(), ABOVE_NORMAL_PRIORITY_CLASS) } != 0;
        tracing::warn!("process priority class above normal (VICTORY_PRIORITY=above): {}", if ok { "set" } else { "refused" });
    }
    if std::env::var_os("VICTORY_THREAD_MAP").is_some_and(|v| v == "1") {
        let _ = std::thread::Builder::new().name("victory-thread-map".into()).spawn(|| {
            let mut before = std::collections::HashMap::new();
            loop {
                std::thread::sleep(std::time::Duration::from_secs(60));
                for line in map(&mut before) {
                    tracing::warn!("{line}");
                }
            }
        });
    }
}

/// The processors helper threads take in turn, or none.
fn order() -> &'static [u32] {
    static ORDER: OnceLock<Vec<u32>> = OnceLock::new();
    ORDER.get_or_init(|| {
        let v = std::env::var("VICTORY_IDEAL").unwrap_or_default();
        let n = std::thread::available_parallelism().map_or(1, |n| n.get()) as u32;
        let order = parse(v.trim(), n);
        if !order.is_empty() {
            tracing::warn!("helper threads' ideal processors in turn (VICTORY_IDEAL): {order:?}");
        }
        order
    })
}

// "1": every processor from the last down; a list: those below `n`
fn parse(v: &str, n: u32) -> Vec<u32> {
    match v {
        "" | "0" => Vec::new(),
        "1" => (0..n).rev().collect(),
        list => list.split(',').filter_map(|p| p.trim().parse().ok()).filter(|&p| p < n).collect(),
    }
}

/// A helper thread of ours starts: its soft ideal processor, if asked.
pub(crate) fn helper() {
    let order = order();
    if order.is_empty() {
        return;
    }
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let processor = order[NEXT.fetch_add(1, Relaxed) % order.len()];
    // SAFETY: this thread's pseudo handle; a hint the scheduler may pass over.
    unsafe {
        windows_sys::Win32::System::Threading::SetThreadIdealProcessor(
            windows_sys::Win32::System::Threading::GetCurrentThread(),
            processor,
        );
    }
}

// the process's threads busy since the last map, busiest first
fn map(before: &mut std::collections::HashMap<u32, u64>) -> Vec<String> {
    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, INVALID_HANDLE_VALUE, LocalFree};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next};
    use windows_sys::Win32::System::Kernel::PROCESSOR_NUMBER;
    use windows_sys::Win32::System::SystemInformation::GROUP_AFFINITY;
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcessId, GetThreadDescription, GetThreadGroupAffinity, GetThreadIdealProcessorEx, GetThreadPriority, GetThreadTimes,
        OpenThread, THREAD_QUERY_INFORMATION,
    };
    let us = |t: FILETIME| (u64::from(t.dwHighDateTime) << 32 | u64::from(t.dwLowDateTime)) / 10;
    let mut rows = Vec::new();
    // SAFETY: plain queries on our own process's threads; handles closed, the description freed.
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
        if snap == INVALID_HANDLE_VALUE {
            return Vec::new();
        }
        let pid = GetCurrentProcessId();
        let mut e: THREADENTRY32 = std::mem::zeroed();
        e.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
        let mut more = Thread32First(snap, &mut e) != 0;
        while more {
            if e.th32OwnerProcessID == pid {
                let h = OpenThread(THREAD_QUERY_INFORMATION, 0, e.th32ThreadID);
                if !h.is_null() {
                    let z = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
                    let (mut c, mut x, mut k, mut u) = (z, z, z, z);
                    GetThreadTimes(h, &mut c, &mut x, &mut k, &mut u);
                    let cpu = us(k) + us(u);
                    let was = before.insert(e.th32ThreadID, cpu).unwrap_or(0);
                    let mut name = String::new();
                    let mut text = std::ptr::null_mut();
                    if GetThreadDescription(h, &mut text) >= 0 && !text.is_null() {
                        let len = (0..).take_while(|&i| *text.add(i) != 0).count();
                        name = String::from_utf16_lossy(std::slice::from_raw_parts(text, len));
                        LocalFree(text.cast());
                    }
                    let mut affinity: GROUP_AFFINITY = std::mem::zeroed();
                    GetThreadGroupAffinity(h, &mut affinity);
                    let mut ideal: PROCESSOR_NUMBER = std::mem::zeroed();
                    GetThreadIdealProcessorEx(h, &mut ideal);
                    let priority = GetThreadPriority(h);
                    CloseHandle(h);
                    rows.push((cpu.saturating_sub(was), e.th32ThreadID, name, affinity.Mask, ideal.Number, priority));
                }
            }
            more = Thread32Next(snap, &mut e) != 0;
        }
        CloseHandle(snap);
    }
    rows.sort_by(|a, b| b.0.cmp(&a.0));
    let total = rows.len();
    let busy: Vec<String> = rows
        .iter()
        .filter(|r| r.0 >= 1_000_000)
        .map(|(cpu, id, name, mask, ideal, priority)| {
            let name = if name.is_empty() { "(title, unnamed)" } else { name };
            format!("{id} {name}: {:.1} s, mask {mask:#x}, ideal {ideal}, priority {priority}", *cpu as f64 / 1e6)
        })
        .collect();
    let mut out = vec![format!(
        "thread map (VICTORY_THREAD_MAP): {} of {total} threads ran a second or more in the last minute",
        busy.len()
    )];
    out.extend(busy.into_iter().map(|b| format!("thread map: {b}")));
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn ideal_orders_count_down_or_keep_the_list() {
        assert_eq!(super::parse("1", 4), [3, 2, 1, 0]);
        assert_eq!(super::parse("15, 13,x,40,11", 16), [15, 13, 11]);
        assert!(super::parse("", 16).is_empty() && super::parse("0", 16).is_empty());
    }

    #[test]
    fn the_thread_map_finds_a_busy_named_thread() {
        let worker = std::thread::Builder::new()
            .name("victory-map-test".into())
            .spawn(|| {
                let end = std::time::Instant::now() + std::time::Duration::from_millis(1300);
                let mut x = 0u64;
                while std::time::Instant::now() < end {
                    x = std::hint::black_box(x.wrapping_mul(31).wrapping_add(7));
                }
                std::thread::park_timeout(std::time::Duration::from_secs(5));
            })
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1500));
        let lines = super::map(&mut std::collections::HashMap::new());
        worker.thread().unpark();
        worker.join().unwrap();
        assert!(lines.iter().any(|l| l.contains("victory-map-test") && l.contains("mask 0x")), "{lines:?}");
    }
}
