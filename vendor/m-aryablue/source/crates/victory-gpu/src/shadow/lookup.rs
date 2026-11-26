//! `VICTORY_LOOKUP_FAST=1` (off by default): recording threads find a title
//! buffer's copy under a shared lock of the table, and each thread keeps a
//! memo of its last answers that every change to the table makes stale.

use std::cell::RefCell;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};

use super::{BUFFERS, Shadow, innermost, lookup_locked, loose};

// bumped by every writer of the table, before it changes anything
static GEN: AtomicU64 = AtomicU64::new(1);

pub(super) fn changed() {
    GEN.fetch_add(1, Ordering::SeqCst);
}

/// `VICTORY_LOOKUP_FAST=1`.
pub(super) fn fast() -> bool {
    // live: GEN moves at every table change with the switch off too, so old memos go stale
    static S: crate::switch::Switch = crate::switch::Switch::noted(
        "LOOKUP_FAST",
        "title buffers' copies are found under a shared lock with a memo per thread (VICTORY_LOOKUP_FAST=1)",
    );
    S.on()
}

const SLOTS: usize = 256;

#[derive(Clone)]
enum Hit {
    // no buffer holds the ask: a window outside buffers
    Loose,
    Copy(Weak<Shadow>),
}

struct Memo {
    era: u64,
    address: u64,
    end: u64,
    hit: Hit,
}

thread_local! {
    static MEMO: RefCell<Vec<Option<Memo>>> = RefCell::new((0..SLOTS).map(|_| None).collect());
}

fn slot(address: u64, end: u64) -> usize {
    ((address ^ end.rotate_left(29)).wrapping_mul(0x9e37_79b9_7f4a_7c15) >> 56) as usize % SLOTS
}

fn remember(era: u64, address: u64, end: u64, hit: Hit) {
    let _ = MEMO.try_with(|m| m.borrow_mut()[slot(address, end)] = Some(Memo { era, address, end, hit }));
}

/// `super::lookup` with the switch: the same answer as `lookup_locked`.
pub(super) fn shared(address: u64, len: u64, any: bool) -> Option<Arc<Shadow>> {
    let end = address.checked_add(len.max(1))?;
    // read before the table: a writer after this makes what is found here stale
    let era = GEN.load(Ordering::SeqCst);
    let memo = MEMO
        .try_with(|m| {
            m.borrow()[slot(address, end)]
                .as_ref()
                .filter(|s| s.era == era && s.address == address && s.end == end)
                .map(|s| s.hit.clone())
        })
        .ok()
        .flatten();
    match memo {
        Some(Hit::Loose) => return loose(address, end, len <= 1, any),
        Some(Hit::Copy(copy)) => {
            if let Some(shadow) = copy.upgrade() {
                return Some(shadow);
            }
        }
        None => {}
    }
    let found = {
        let map = BUFFERS.read().unwrap_or_else(|e| e.into_inner());
        match innermost(&map, address, end) {
            None => None,
            Some((_, Some(shadow))) => Some(Some(shadow.clone())),
            // its copy is still to make: under the write lock
            Some((_, None)) => Some(None),
        }
    };
    match found {
        None => {
            remember(era, address, end, Hit::Loose);
            loose(address, end, len <= 1, any)
        }
        Some(Some(shadow)) => {
            remember(era, address, end, Hit::Copy(Arc::downgrade(&shadow)));
            Some(shadow)
        }
        Some(None) => lookup_locked(address, len, any),
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Entry, Shadow, buffers, lookup_locked, unregister};
    use super::shared;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use windows::Win32::Graphics::Direct3D::D3D_FEATURE_LEVEL_11_0;
    use windows::Win32::Graphics::Direct3D12::*;
    use windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC;
    use windows::Win32::Graphics::Dxgi::*;

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

    // a buffer in the table with its copy made; the resource names it apart from other tests'
    fn put(buffer: &ID3D12Resource, resource: usize, start: u64, len: u64) -> Arc<Shadow> {
        let shadow = Arc::new(Shadow::new(buffer.clone(), std::ptr::null_mut(), 0, start, len));
        // in one go: no ask sees it without its copy
        buffers().insert((start, resource), Entry { end: start + len, shadow: Some(shadow.clone()) });
        shadow
    }

    fn same(a: &Option<Arc<Shadow>>, b: &Option<Arc<Shadow>>) -> bool {
        match (a, b) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        }
    }

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
    }

    // nested and side-by-side buffers come and go; every ask finds what the locked lookup finds
    #[test]
    fn the_shared_lookup_finds_what_the_locked_one_finds() {
        let Some(buffer) = warp_buffer() else {
            return;
        };
        const BASE: u64 = 0x7a10_0000_0000;
        const SPAN: u64 = 64 << 20;
        let mut rng = Rng(0x0dd_b1a5_e5ba_d5ee);
        // the outer buffer holds every ask, so no ask goes to the windows outside buffers
        let outer = put(&buffer, 0x51_0000, BASE, SPAN);
        let mut live: Vec<(usize, u64, Arc<Shadow>)> = Vec::new();
        for round in 0..400 {
            if live.len() < 40 && (live.is_empty() || rng.next() % 3 != 0) {
                let start = BASE + (rng.next() % (SPAN / 2)) & !0xff;
                let len = 256 + rng.next() % (4 << 20);
                let resource = 0x51_0001 + round;
                live.push((resource, start, put(&buffer, resource, start, len)));
            } else {
                let (resource, start, _) = live.swap_remove(rng.next() as usize % live.len());
                unregister(resource, start);
            }
            for _ in 0..50 {
                let address = BASE + rng.next() % (SPAN - (1 << 20));
                let len = rng.next() % 4096;
                // twice: the second answer comes from the memo
                let want = lookup_locked(address, len, false);
                assert!(same(&shared(address, len, false), &want), "round {round} at {address:#x}+{len:#x}");
                assert!(same(&shared(address, len, false), &want), "round {round} again");
            }
        }
        for (resource, start, _) in live {
            unregister(resource, start);
        }
        unregister(0x51_0000, BASE);
        drop(outer);
    }

    // while a buffer inside another comes and goes, an ask after a change sees the change
    #[test]
    fn asks_after_a_change_never_see_the_table_before_it() {
        nested_churn(true);
    }

    // the same with each removed copy freed at once: a stale memo must not hand it out
    #[test]
    fn a_removed_and_freed_copy_is_never_found() {
        nested_churn(false);
    }

    fn nested_churn(keep: bool) {
        let Some(buffer) = warp_buffer() else {
            return;
        };
        let base: u64 = if keep { 0x7a20_0000_0000 } else { 0x7a28_0000_0000 };
        let tag = if keep { 0x52_0000 } else { 0x54_0000 };
        let outer = put(&buffer, tag, base, 16 << 20);
        let inner_at = base + (4 << 20);
        // even steps: the inner buffer is gone; odd steps: it is there. weak: the allocation
        // stays while compared, so a freed copy's address is never taken by another
        let step = Arc::new(AtomicU64::new(0));
        let inners: Arc<std::sync::Mutex<Vec<Option<std::sync::Weak<Shadow>>>>> = Arc::new(std::sync::Mutex::new(vec![None]));
        let stop = Arc::new(AtomicBool::new(false));
        let readers: Vec<_> = (0..4)
            .map(|_| {
                let (step, inners, stop, outer) = (step.clone(), inners.clone(), stop.clone(), outer.clone());
                std::thread::spawn(move || {
                    let mut checked = 0u64;
                    while !stop.load(Ordering::SeqCst) {
                        let s = step.load(Ordering::SeqCst);
                        let found = shared(inner_at + 64, 128, false).expect("the outer buffer at least");
                        let after = step.load(Ordering::SeqCst);
                        let found_weak = Arc::downgrade(&found);
                        let inners = inners.lock().unwrap();
                        // the change after `after` may be under way: its entry is noted before it starts
                        let last = (after as usize + 1).min(inners.len() - 1);
                        let ok = (s as usize..=last).any(|k| match &inners[k] {
                            Some(inner) => std::sync::Weak::ptr_eq(&found_weak, inner),
                            None => Arc::ptr_eq(&found, &outer),
                        });
                        assert!(ok, "steps {s}..={last}");
                        drop(inners);
                        drop(found);
                        checked += 1;
                    }
                    checked
                })
            })
            .collect();
        let mut kept = Vec::new();
        for k in 1..=400u64 {
            let resource = tag + 1 + k as usize;
            if k % 2 == 1 {
                let inner = Arc::new(Shadow::new(buffer.clone(), std::ptr::null_mut(), 0, inner_at, 1 << 20));
                inners.lock().unwrap().push(Some(Arc::downgrade(&inner)));
                buffers().insert((inner_at, resource), Entry { end: inner_at + (1 << 20), shadow: Some(inner.clone()) });
                if keep {
                    kept.push(inner);
                }
            } else {
                inners.lock().unwrap().push(None);
                unregister(resource - 1, inner_at);
            }
            step.store(k, Ordering::SeqCst);
            std::thread::yield_now();
        }
        stop.store(true, Ordering::SeqCst);
        let checked: u64 = readers.into_iter().map(|r| r.join().expect("a reader failed")).sum();
        assert!(checked > 0);
        unregister(tag, base);
    }

    // timings for the report: cargo test --release lookup_contention -- --ignored --nocapture
    #[test]
    #[ignore]
    fn lookup_contention() {
        let Some(buffer) = warp_buffer() else {
            return;
        };
        const BASE: u64 = 0x7a30_0000_0000;
        let mut held = Vec::new();
        // 300 buffers side by side, a few nested, as a frame's bindings
        for i in 0..300u64 {
            held.push(put(&buffer, 0x53_0000 + i as usize, BASE + i * (1 << 20), 1 << 20));
        }
        for i in 0..30u64 {
            held.push(put(&buffer, 0x53_1000 + i as usize, BASE + i * (10 << 20) + 4096, 64 << 10));
        }
        let asks: Arc<Vec<(u64, u64)>> = Arc::new({
            let mut rng = Rng(77);
            (0..4400).map(|_| (BASE + rng.next() % (300 << 20) - 256, 256)).map(|(a, l)| (a.max(BASE), l)).collect()
        });
        // the first pass warms up; the second is printed
        for (pass, name, fast) in [(0, "locked", false), (0, "shared", true), (1, "locked", false), (1, "shared", true)] {
            for threads in [1usize, 5] {
                let t = std::time::Instant::now();
                let workers: Vec<_> = (0..threads)
                    .map(|_| {
                        let asks = asks.clone();
                        std::thread::spawn(move || {
                            for _ in 0..20 {
                                for &(a, l) in asks.iter() {
                                    let s = if fast { shared(a, l, false) } else { lookup_locked(a, l, false) };
                                    std::hint::black_box(s);
                                }
                            }
                        })
                    })
                    .collect();
                workers.into_iter().for_each(|w| w.join().unwrap());
                let per = t.elapsed().as_secs_f64() * 1000.0 / 20.0;
                if pass == 1 {
                    println!("{name}, {threads} threads x 4,400 asks: {per:.3} ms a frame (wall)");
                }
            }
        }
        for i in 0..300u64 {
            unregister(0x53_0000 + i as usize, BASE + i * (1 << 20));
        }
        for i in 0..30u64 {
            unregister(0x53_1000 + i as usize, BASE + i * (10 << 20) + 4096);
        }
    }
}
