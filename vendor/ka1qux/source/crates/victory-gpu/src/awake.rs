//! Keeps the card's clocks up between submissions (`VICTORY_CARD_AWAKE=1`; off
//! by default).
//!
//! The card stands idle for a good part of every frame, dozens of times for a
//! millisecond or two each, while the CPU prepares the next submission. The
//! driver takes that for a light load and lowers the clocks, and what comes
//! next starts slow: a captured frame whose lists take 28.6 ms sent in one go
//! took 37.8 ms with 1.2 ms between its submissions, as the title sends them,
//! and 28.6 ms again with the card kept busy (the measurement of the fork this
//! is taken from, `avatarmanz/VIctory-7900XTX` commit `5f86fa3`).
//!
//! One group of 64 threads counts on a compute queue of its own, over and
//! over, about 2 ms each time: it takes one or two of the card's few thousand
//! wave slots, and the card is never idle. Nothing is sent once no submission
//! has gone out for a while (a menu, a minimized window), nothing once the
//! device is removed, and the thread ends, with what it sent run, at the end
//! of the process or when the host fails.
//!
//! `VICTORY_CARD_AWAKE=turns`: twenty seconds on, twenty off, to compare the
//! two in one run (the log says when it turns).

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::CloseHandle;
use windows::Win32::Graphics::Direct3D12::*;
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
use windows::core::Interface;

/// About 2 ms at full clocks: some 500 sends a second.
const COUNT: &str = r#"
#define RS "UAV(u0)"
RWByteAddressBuffer sum : register(u0);
[RootSignature(RS)]
[numthreads(64, 1, 1)]
void main(uint3 id : SV_DispatchThreadID) {
    uint x = id.x;
    [loop] for (uint i = 0; i < 400000u; i++) x = x * 1664525u + 1013904223u;
    sum.Store(id.x * 4u, x);
}
"#;

/// Quiet this long after the last submission.
const QUIET_MS: u64 = 250;

/// With `VICTORY_CARD_AWAKE=turns`: this long on, this long off.
const TURN_MS: u64 = 20_000;

/// The longest the end of the process waits for the thread.
const END_WAIT: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Off,
    On,
    /// On and off by turns, to compare the two in one run.
    ByTurns,
}

fn mode() -> Mode {
    static MODE: OnceLock<Mode> = OnceLock::new();
    *MODE.get_or_init(|| match std::env::var("VICTORY_CARD_AWAKE").as_deref() {
        Ok("1") => Mode::On,
        Ok("turns") => Mode::ByTurns,
        _ => Mode::Off,
    })
}

/// When a submission was last sent ([`crate::host::now_ms`] plus one; 0:
/// never).
static LAST: AtomicU64 = AtomicU64::new(0);
/// Sends of the counting group since the last frame-rate line, and all of
/// them.
static LINE_SENDS: AtomicU64 = AtomicU64::new(0);
static ALL_SENDS: AtomicU64 = AtomicU64::new(0);
/// The host failed or the process ends: nothing more goes to the queue.
static HALTED: AtomicBool = AtomicBool::new(false);
/// The thread, kept until the process ends.
static AWAKE: Mutex<Option<Awake>> = Mutex::new(None);

/// Whether the counting group should run now: submissions are going out
/// (and, by turns, this is an "on" turn).
fn wanted() -> bool {
    let (last, now) = (LAST.load(Relaxed), crate::host::now_ms());
    last != 0 && now + 1 < last + QUIET_MS && (mode() != Mode::ByTurns || now / TURN_MS % 2 == 1)
}

unsafe extern "C" {
    fn atexit(f: extern "C" fn()) -> i32;
}

/// At the end of the process (registered after the loader's own exit
/// function, so run before it ends the other threads): what was sent has run
/// before the device goes.
extern "C" fn at_exit() {
    halt();
    let awake = AWAKE.try_lock().ok().and_then(|mut a| a.take());
    drop(awake);
}

/// Nothing more goes to the queue: the host failed (`crate::host::fail`) or
/// the process is ending.
pub(crate) fn halt() {
    HALTED.store(true, Relaxed);
}

/// A submission of ours went to `device`'s drawing queue.
pub(crate) fn sent(device: &ID3D12Device) {
    if mode() == Mode::Off || HALTED.load(Relaxed) {
        return;
    }
    let now = crate::host::now_ms();
    let before = LAST.swap(now + 1, Relaxed);
    if mode() == Mode::ByTurns && before.saturating_sub(1) / TURN_MS != now / TURN_MS {
        tracing::info!(on = now / TURN_MS % 2 == 1, "the card is kept awake, by turns (VICTORY_CARD_AWAKE=turns)");
    }
    static STARTED: std::sync::Once = std::sync::Once::new();
    STARTED.call_once(|| {
        let Some(awake) = start(device, wanted) else {
            tracing::warn!("no thread to keep the card awake (VICTORY_CARD_AWAKE)");
            return;
        };
        *AWAKE.lock().unwrap_or_else(|e| e.into_inner()) = Some(awake);
        // SAFETY: a plain C function of ours, run at the process's end.
        unsafe { atexit(at_exit) };
        tracing::info!(
            "the card is kept awake: a group of 64 threads counting on a compute queue of its own, about 2 ms at a time, \
             while submissions go out, none once {QUIET_MS} ms pass without one (VICTORY_CARD_AWAKE)"
        );
    });
}

/// For the frame-rate line: how often the counting group was sent.
pub(crate) fn line(frames: f64) {
    if mode() == Mode::Off {
        return;
    }
    let sends = LINE_SENDS.swap(0, Relaxed);
    tracing::info!(
        "the card kept awake per frame (VICTORY_CARD_AWAKE): {:.1} sends of the counting group ({} so far)",
        sends as f64 / frames,
        ALL_SENDS.load(Relaxed)
    );
}

/// The thread that keeps the card busy; it ends when this is dropped.
pub(crate) struct Awake {
    stop: Arc<AtomicBool>,
    sends: Arc<AtomicU64>,
    failed: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Awake {
    /// How many times the counting group was sent.
    #[cfg_attr(not(test), allow(dead_code))]
    fn sends(&self) -> u64 {
        self.sends.load(Relaxed)
    }

    /// Whether the thread met an error and ended (it says what in the log).
    #[cfg_attr(not(test), allow(dead_code))]
    fn failed(&self) -> bool {
        self.failed.load(Relaxed)
    }
}

impl Drop for Awake {
    fn drop(&mut self) {
        self.stop.store(true, Relaxed);
        let Some(thread) = self.thread.take() else {
            return;
        };
        // The thread waits for what it sent, up to a second; a card that
        // never answers is not waited for longer than that here.
        let waiting = Instant::now();
        while !thread.is_finished() && waiting.elapsed() < END_WAIT {
            std::thread::sleep(Duration::from_millis(1));
        }
        if thread.is_finished() {
            let _ = thread.join();
        }
    }
}

/// Keeps the card of `device` busy while `wanted`.
pub(crate) fn start(device: &ID3D12Device, wanted: impl Fn() -> bool + Send + 'static) -> Option<Awake> {
    let stop = Arc::new(AtomicBool::new(false));
    let sends = Arc::new(AtomicU64::new(0));
    let failed = Arc::new(AtomicBool::new(false));
    let (device, ended, counted, broke) = (device.clone(), stop.clone(), sends.clone(), failed.clone());
    let thread = std::thread::Builder::new()
        .name("victory-card-awake".into())
        .spawn(move || {
            if let Err(e) = run(&device, &wanted, &ended, &counted) {
                broke.store(true, Relaxed);
                tracing::warn!(error = %e, "the card is not kept awake any longer");
            }
        })
        .ok()?;
    Some(Awake {
        stop,
        sends,
        failed,
        thread: Some(thread),
    })
}

fn run(
    device: &ID3D12Device,
    wanted: &dyn Fn() -> bool,
    stop: &AtomicBool,
    sends: &AtomicU64,
) -> windows::core::Result<()> {
    let code = crate::host::compile(COUNT, windows::core::s!("main"), windows::core::s!("cs_5_1"))?;
    // SAFETY: plain object creation; the code lives through the calls, the
    // root signature is inside it. Two closed lists, sent in turn, whose
    // objects live until what was sent last has run.
    unsafe {
        let root: ID3D12RootSignature = device.CreateRootSignature(0, &code)?;
        let pso: ID3D12PipelineState = device.CreateComputePipelineState(&D3D12_COMPUTE_PIPELINE_STATE_DESC {
            pRootSignature: std::mem::transmute_copy(&root),
            CS: D3D12_SHADER_BYTECODE {
                pShaderBytecode: code.as_ptr().cast(),
                BytecodeLength: code.len(),
            },
            ..Default::default()
        })?;
        let queue: ID3D12CommandQueue = device.CreateCommandQueue(&D3D12_COMMAND_QUEUE_DESC {
            Type: D3D12_COMMAND_LIST_TYPE_COMPUTE,
            Priority: D3D12_COMMAND_QUEUE_PRIORITY_NORMAL.0,
            ..Default::default()
        })?;
        let mut sum: Option<ID3D12Resource> = None;
        device.CreateCommittedResource(
            &D3D12_HEAP_PROPERTIES {
                Type: D3D12_HEAP_TYPE_DEFAULT,
                ..Default::default()
            },
            D3D12_HEAP_FLAG_NONE,
            &D3D12_RESOURCE_DESC {
                Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
                Width: 256,
                Height: 1,
                DepthOrArraySize: 1,
                MipLevels: 1,
                SampleDesc: windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
                Flags: D3D12_RESOURCE_FLAG_ALLOW_UNORDERED_ACCESS,
                ..Default::default()
            },
            D3D12_RESOURCE_STATE_COMMON,
            None,
            &mut sum,
        )?;
        let sum = sum.ok_or_else(windows::core::Error::empty)?;
        let allocator: ID3D12CommandAllocator = device.CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_COMPUTE)?;
        let mut lists = Vec::new();
        for _ in 0..2 {
            let list: ID3D12GraphicsCommandList =
                device.CreateCommandList(0, D3D12_COMMAND_LIST_TYPE_COMPUTE, &allocator, &pso)?;
            list.SetComputeRootSignature(&root);
            list.SetComputeRootUnorderedAccessView(0, sum.GetGPUVirtualAddress());
            list.Dispatch(1, 1, 1);
            list.Close()?;
            lists.push(Some(list.cast::<ID3D12CommandList>()?));
        }
        let fence: ID3D12Fence = device.CreateFence(0, D3D12_FENCE_FLAG_NONE)?;
        let event = CreateEventW(None, false, false, None)?;
        let mut value = 0u64;
        let mut result = Ok(());
        while !stop.load(Relaxed) && !HALTED.load(Relaxed) && result.is_ok() {
            if !wanted() {
                std::thread::sleep(Duration::from_millis(2));
                continue;
            }
            // A removed device takes nothing more.
            result = device.GetDeviceRemovedReason();
            if result.is_err() {
                break;
            }
            // Never more than two on the way: each list is sent again only
            // after its last run has ended.
            if value >= 2 && fence.GetCompletedValue() + 2 <= value {
                result = fence.SetEventOnCompletion(value - 1, event);
                if result.is_ok() {
                    WaitForSingleObject(event, 1000);
                    result = device.GetDeviceRemovedReason();
                }
                continue;
            }
            queue.ExecuteCommandLists(&lists[(value % 2) as usize..][..1]);
            value += 1;
            result = queue.Signal(&fence, value);
            sends.fetch_add(1, Relaxed);
            LINE_SENDS.fetch_add(1, Relaxed);
            ALL_SENDS.fetch_add(1, Relaxed);
        }
        // What was sent runs to its end before the objects go (not for a
        // removed device, which runs nothing, nor for longer than a second).
        if device.GetDeviceRemovedReason().is_ok()
            && fence.GetCompletedValue() < value
            && fence.SetEventOnCompletion(value, event).is_ok()
        {
            WaitForSingleObject(event, 1000);
        }
        let _ = CloseHandle(event);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_counting_group_runs_while_wanted_and_ends_with_what_it_sent() {
        let Some(host) = crate::host::host() else {
            eprintln!("no host device: skipped");
            return;
        };
        let awake = start(&host.device, || true).expect("a thread");
        let waiting = Instant::now();
        while awake.sends() < 3 && !awake.failed() && waiting.elapsed() < Duration::from_secs(20) {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!awake.failed(), "the thread met an error");
        assert!(awake.sends() >= 3, "sent only {} times", awake.sends());
        let ended = Instant::now();
        drop(awake);
        assert!(ended.elapsed() < Duration::from_secs(5), "dropping it took {:?}", ended.elapsed());
        // SAFETY: a plain query.
        assert!(unsafe { host.device.GetDeviceRemovedReason() }.is_ok());
    }

    #[test]
    fn nothing_is_sent_while_not_wanted() {
        let Some(host) = crate::host::host() else {
            eprintln!("no host device: skipped");
            return;
        };
        let awake = start(&host.device, || false).expect("a thread");
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(awake.sends(), 0);
        assert!(!awake.failed());
        drop(awake);
    }
}
