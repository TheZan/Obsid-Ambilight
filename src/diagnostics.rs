//! Log file setup: one file per process under `logs\`, rotated at start-up
//! when it exceeds [`MAX_LOG_BYTES`], plus a panic hook that logs panics.

use std::fs::{self, OpenOptions};
use std::io;
use std::path::Path;
use std::sync::Mutex;

pub const MAX_LOG_BYTES: u64 = 1024 * 1024;

/// Moves `path` to `<path>.1` (replacing an older copy) when it is larger than `max`.
pub fn rotate(path: &Path, max: u64) -> io::Result<()> {
    match fs::metadata(path) {
        Ok(m) if m.len() > max => {
            let mut old = path.as_os_str().to_owned();
            old.push(".1");
            let _ = fs::remove_file(&old);
            fs::rename(path, old)
        }
        Ok(_) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// Sends `tracing` output to `path` (appending) and installs the panic hook.
pub fn init_logging(path: &Path, level: tracing::Level) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    rotate(path, MAX_LOG_BYTES)?;
    let file = OpenOptions::new().create(true).append(true).open(path)?;
    tracing_subscriber::fmt()
        .with_writer(Mutex::new(file))
        .with_ansi(false)
        .with_target(false)
        .with_max_level(level)
        .try_init()
        .map_err(|e| io::Error::other(e.to_string()))?;
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        tracing::error!("panic: {info}");
        previous(info);
    }));
    Ok(())
}
