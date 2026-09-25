//! Reserved automatic chart files (DESIGN §6.6).
//!
//! [`TempChart`] reserves a fresh private path with `create_new` and keeps
//! the resulting handle open through HTML writing. Writing through the
//! reserved descriptor — never by reopening the path — closes the
//! reservation/output replacement window: a symlink swapped in after
//! reservation cannot redirect the write (A006).

use std::collections::hash_map::RandomState;
use std::fs::{File, OpenOptions};
use std::hash::BuildHasher;
use std::io::Write;
use std::path::{Path, PathBuf};

/// An automatic chart file whose descriptor is held from reservation through
/// writing, so the bytes always land in the reserved file even if the path
/// is replaced between reservation and output.
pub(crate) struct TempChart {
    file: File,
    path: PathBuf,
}

impl TempChart {
    /// The reserved path. Valid for display and session teardown, never for
    /// reopening to write.
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// Writes the chart HTML through the reserved descriptor.
    pub(crate) fn write_html(&mut self, html: &str) -> Result<(), String> {
        self.file
            .write_all(html.as_bytes())
            .map_err(|error| format!("failed to write chart file: {error}"))?;
        self.file
            .sync_all()
            .map_err(|error| format!("failed to persist chart file: {error}"))
    }
}

/// Reserves a fresh, private automatic chart file and records it for session
/// teardown. `create_new` makes the reservation atomic, so an existing
/// symlink or file is never followed or replaced.
pub(crate) fn reserve_temp_chart() -> Result<TempChart, String> {
    let temp = std::env::temp_dir();
    for attempt in 0..16_u64 {
        let nonce = RandomState::new().hash_one((std::process::id(), attempt));
        let path = temp.join(format!("saya-chart-{nonce:016x}.html"));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&path) {
            Ok(file) => {
                super::cleanup::record_temp_chart(&path);
                return Ok(TempChart { file, path });
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("failed to create chart file: {error}")),
        }
    }
    Err("failed to allocate a unique chart file".into())
}

/// Test-only serialization for the shared session chart registry: every
/// chart test takes this before touching recorded files.
#[cfg(test)]
pub(crate) fn lock_charts_for_test() -> std::sync::MutexGuard<'static, ()> {
    use std::sync::Mutex;
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap()
}

#[cfg(test)]
#[path = "temp_chart_tests.rs"]
mod tests;
