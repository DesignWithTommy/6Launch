//! A captured frame's draws and dispatches as JSON lines, for replaying
//! one of them without the game: with `VICTORY_TRACE_FRAME=1` (and
//! `VICTORY_CAPTURE=<folder>`), every frame `capture::listed` names is
//! written to `<folder>/frame-<n>.jsonl`, one line per draw, dispatch or
//! indirect execution, in recording order:
//!
//! `{"frame":n,"seq":i,"op":"draw","args":[..],"ran":true,"pipeline":"0x..",
//!   "stages":["<hash>",..],"views":[{"slot":s,"kind":"srv","address":"0x..",
//!   "left":bytes},..],"targets":[{"resource":"0x..","width":w,"height":h,
//!   "format":f},..]}`
//!
//! `views` are the root views the stages read (slot numbers as the title
//! set them; pointer slots from `victory_shader::emit::POINTER_SLOT` on,
//! address from root constants), `left` the bytes of the title buffer's
//! host copy from the address on (0: none). Only in builds with
//! diagnostics.

use std::cell::RefCell;
use std::io::Write;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

/// The frame being traced, if any (`VICTORY_TRACE_FRAME` set and the frame
/// being recorded listed for capture).
pub(crate) fn active() -> Option<u64> {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if !*ON.get_or_init(|| std::env::var_os("VICTORY_TRACE_FRAME").is_some_and(|v| v != "0")) {
        return None;
    }
    let frame = crate::queue::presented() + 1;
    crate::capture::listed(frame).then_some(frame)
}

thread_local! {
    static VIEWS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// While binding: a root view a stage reads.
pub(crate) fn view(slot: u8, kind: &str, address: u64) {
    if active().is_none() {
        return;
    }
    let left = crate::shadow::find(address, 1).map_or(0, |s| s.left(address));
    VIEWS.with(|v| {
        v.borrow_mut().push(format!(
            r#"{{"slot":{slot},"kind":"{kind}","address":"{address:#x}","left":{left}}}"#
        ));
    });
}

/// Drops views noted for a bind that is not followed by an op.
pub(crate) fn clear() {
    VIEWS.with(|v| v.borrow_mut().clear());
}

/// Open trace file: the frame it is for.
static FILE: Mutex<Option<(u64, std::fs::File)>> = Mutex::new(None);
static SEQ: AtomicU64 = AtomicU64::new(0);

/// After a draw, dispatch or indirect execution was recorded (or refused,
/// `ran` false): one line.
pub(crate) fn op(
    kind: &str,
    args: &[u64],
    ran: bool,
    pipeline: usize,
    stages: &[String],
    targets: &[(usize, u64, u32, i32)],
) {
    let Some(frame) = active() else {
        return;
    };
    let views = VIEWS.with(|v| std::mem::take(&mut *v.borrow_mut()));
    let Some(folder) = crate::capture::folder() else {
        return;
    };
    let mut file = FILE.lock().unwrap_or_else(|e| e.into_inner());
    if file.as_ref().is_none_or(|(f, _)| *f != frame) {
        let path = folder.join(format!("frame-{frame}.jsonl"));
        match std::fs::File::create(&path) {
            Ok(f) => {
                tracing::info!(path = %path.display(), "frame trace");
                *file = Some((frame, f));
                SEQ.store(0, Ordering::Relaxed);
            }
            Err(e) => {
                tracing::warn!(error = %e, path = %path.display(), "no frame trace");
                *file = None;
                return;
            }
        }
    }
    let Some((_, f)) = file.as_mut() else { return };
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let args: Vec<String> = args.iter().map(u64::to_string).collect();
    let stages: Vec<String> = stages.iter().map(|h| format!("\"{h}\"")).collect();
    let targets: Vec<String> = targets
        .iter()
        .map(|&(r, w, h, fmt)| {
            format!(r#"{{"resource":"{r:#x}","width":{w},"height":{h},"format":{fmt}}}"#)
        })
        .collect();
    let _ = writeln!(
        f,
        r#"{{"frame":{frame},"seq":{seq},"op":"{kind}","args":[{}],"ran":{ran},"pipeline":"{pipeline:#x}","stages":[{}],"views":[{}],"targets":[{}]}}"#,
        args.join(","),
        stages.join(","),
        views.join(","),
        targets.join(",")
    );
}
