//! Per-user locations under `%APPDATA%\ObsidAmbilight`.

use std::io;
use std::path::PathBuf;

pub fn app_dir() -> io::Result<PathBuf> {
    std::env::var_os("APPDATA")
        .map(|p| PathBuf::from(p).join("ObsidAmbilight"))
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "APPDATA is not set"))
}

pub fn config_path() -> io::Result<PathBuf> {
    Ok(app_dir()?.join("config.toml"))
}

pub fn logs_dir() -> io::Result<PathBuf> {
    Ok(app_dir()?.join("logs"))
}
