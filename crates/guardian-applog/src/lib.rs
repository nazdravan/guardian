//! Log files of the Guardian desktop programs (console, setup): rotated at
//! start (at most two old files), plus a panic hook that writes to the log.

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};

/// Rotation threshold.
pub const MAX_LOG: u64 = 1024 * 1024;

fn with_suffix(path: &Path, n: u32) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(format!(".{n}"));
    PathBuf::from(s)
}

/// If `path` is larger than `max_bytes`: `.1` becomes `.2`, `path` becomes
/// `.1`. A missing file or directory is fine.
pub fn rotate(path: &Path, max_bytes: u64) -> std::io::Result<()> {
    match std::fs::metadata(path) {
        Ok(m) if m.len() > max_bytes => {
            let one = with_suffix(path, 1);
            if one.exists() {
                std::fs::rename(&one, with_suffix(path, 2))?;
            }
            std::fs::rename(path, one)
        }
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// Logs to `path` (`info`, or `RUST_LOG`); `false` when only stderr is used.
pub fn init_logging(path: &Path) -> bool {
    let filter = || {
        tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"))
    };
    let opened = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| rotate(path, MAX_LOG))
        .and_then(|()| {
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
        });
    match opened {
        Ok(f) => {
            let _ = tracing_subscriber::fmt()
                .with_env_filter(filter())
                .with_writer(std::sync::Mutex::new(f))
                .with_ansi(false)
                .try_init();
            true
        }
        Err(e) => {
            let _ = tracing_subscriber::fmt()
                .with_env_filter(filter())
                .with_writer(std::io::stderr)
                .try_init();
            tracing::warn!(
                "cannot write {}: {e}; logging to stderr only",
                path.display()
            );
            false
        }
    }
}

/// Writes panics to the log (message and location).
pub fn log_panics() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let msg = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown panic".into());
        let at = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_default();
        tracing::error!("panic at {at}: {msg}");
        previous(info);
    }));
}
