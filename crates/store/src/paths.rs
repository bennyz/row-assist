//! Data locations. Everything lives under one directory:
//! `$ROW_ASSIST_DATA`, else `$XDG_DATA_HOME/row-assist`, else
//! `~/.local/share/row-assist`. Absolute, so it doesn't move with the working
//! directory (v1's relative `sqlite://row_assist.db` did).

use std::path::PathBuf;

pub fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("ROW_ASSIST_DATA") {
        return dir.into();
    }
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("row-assist")
}

pub fn db_path() -> PathBuf {
    data_dir().join("row-assist.db")
}

pub fn sessions_dir() -> PathBuf {
    data_dir().join("sessions")
}
