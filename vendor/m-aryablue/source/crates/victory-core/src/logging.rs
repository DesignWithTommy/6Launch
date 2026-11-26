//! One tracing setup for every binary.
//!
//! Filter with `VICTORY_LOG` (falls back to `RUST_LOG`), e.g.
//! `VICTORY_LOG=victory_loader=trace,info`. Debug and trace output exist
//! only in builds with the `diagnostics` feature; a plain release build
//! logs info and above whatever the filter asks for.
//!
//! `VICTORY_LOG_FILE=<path>` writes the log to that file through a buffer
//! flushed every half second, not line by line to stderr.

use tracing_subscriber::EnvFilter;
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// The most verbose level this build can log.
const CAP: LevelFilter = if cfg!(feature = "diagnostics") {
    LevelFilter::TRACE
} else {
    LevelFilter::INFO
};

pub fn init(default_level: &str) {
    if let Some(file) = std::env::var_os("VICTORY_LOG_FILE").and_then(|path| Buffered::open(path.as_ref()).ok()) {
        return init_with(default_level, file, false);
    }
    init_with(default_level, std::io::stderr, true);
}

fn init_with<W>(default_level: &str, writer: W, ansi: bool)
where
    W: for<'w> tracing_subscriber::fmt::MakeWriter<'w> + Send + Sync + 'static,
{
    let filter = std::env::var("VICTORY_LOG")
        .or_else(|_| std::env::var("RUST_LOG"))
        .ok()
        .and_then(|v| EnvFilter::try_new(v).ok())
        .unwrap_or_else(|| EnvFilter::new(default_level));
    let _ = tracing_subscriber::registry()
        .with(CAP)
        .with(filter)
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(true)
                .with_ansi(ansi)
                .with_writer(writer),
        )
        .try_init();
}

/// A log file written through a buffer; a thread flushes it every half
/// second, so a killed run loses at most that much.
#[derive(Clone)]
pub struct Buffered(std::sync::Arc<std::sync::Mutex<std::io::BufWriter<std::fs::File>>>);

impl Buffered {
    pub fn open(path: &std::path::Path) -> std::io::Result<Self> {
        let file = std::fs::File::create(path)?;
        let this = Self(std::sync::Arc::new(std::sync::Mutex::new(std::io::BufWriter::with_capacity(1 << 20, file))));
        let weak = std::sync::Arc::downgrade(&this.0);
        let _ = std::thread::Builder::new().name("victory-log-flush".into()).spawn(move || {
            while let Some(out) = weak.upgrade() {
                if let Ok(mut out) = out.lock() {
                    let _ = std::io::Write::flush(&mut *out);
                }
                drop(out);
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
        });
        Ok(this)
    }
}

impl std::io::Write for Buffered {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().map_or(Ok(bytes.len()), |mut out| out.write(bytes))
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0.lock().map_or(Ok(()), |mut out| out.flush())
    }
}

impl<'w> tracing_subscriber::fmt::MakeWriter<'w> for Buffered {
    type Writer = Self;

    fn make_writer(&'w self) -> Self {
        self.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::Buffered;
    use std::io::Write;

    #[test]
    fn a_buffered_log_reaches_its_file_without_a_flush() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("run.log");
        let mut log = Buffered::open(&path).unwrap();
        log.write_all(b"5.3 fps, 187.1 ms/frame
").unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::fs::read(&path).unwrap().is_empty() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert_eq!(std::fs::read(&path).unwrap(), b"5.3 fps, 187.1 ms/frame
");
    }

    #[test]
    fn the_lean_filter_keeps_the_frame_line_alone() {
        use tracing_subscriber::layer::SubscriberExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lean.log");
        let log = Buffered::open(&path).unwrap();
        let subscriber = tracing_subscriber::registry()
            .with(tracing_subscriber::EnvFilter::new("warn,victory_gpu::host::frame=info"))
            .with(tracing_subscriber::fmt::layer().with_ansi(false).with_writer(log.clone()));
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(target: "victory_gpu::host", "a stats line");
            tracing::info!(target: "victory_gpu::host::frame", "5.3 fps, 187.1 ms/frame");
            tracing::warn!(target: "victory_gpu::pipeline", "a warning");
        });
        log.clone().flush().unwrap();
        let text = String::from_utf8(std::fs::read(&path).unwrap()).unwrap();
        assert!(text.contains("5.3 fps") && text.contains("a warning") && !text.contains("a stats line"), "{text}");
    }
}
