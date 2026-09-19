//! Session-scoped lifecycle for chart temp files (DESIGN §6.6).
//!
//! `render_chart` writes `saya-chart-*.html` into the temp directory; without
//! lifecycle management they accumulate forever. Every auto-generated path is
//! recorded at write time and removed when the session ends. Files the user
//! names explicitly (the TUI `/chart <path>` form) are never recorded and are
//! therefore never deleted.

use std::collections::hash_map::RandomState;
use std::hash::BuildHasher;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::{fs::OpenOptions, io};

static SESSION_CHART_FILES: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// Records a chart temp file written this session so the session teardown can
/// remove it. Best-effort: a poisoned registry only means the file outlives
/// the session, exactly as it did before the registry existed.
pub(crate) fn record_temp_chart(path: &Path) {
    if let Ok(mut files) = SESSION_CHART_FILES.lock() {
        files.push(path.to_path_buf());
    }
}

/// Creates and records an automatic chart file with a fresh, private name.
///
/// `create_new` makes the path reservation atomic, so an existing symlink or
/// file is never followed or replaced. The caller writes the HTML to the
/// returned path and may rely on session teardown to remove it.
pub(crate) fn create_temp_chart() -> Result<PathBuf, String> {
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
            Ok(_) => {
                record_temp_chart(&path);
                return Ok(path);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("failed to create chart file: {error}")),
        }
    }
    Err("failed to allocate a unique chart file".into())
}

/// Deletes the given chart files, returning how many were removed. Missing
/// files are not an error — cleanup is best-effort by contract.
pub(crate) fn cleanup(paths: &[PathBuf]) -> usize {
    paths
        .iter()
        .filter(|path| std::fs::remove_file(path).is_ok())
        .count()
}

/// Removes every chart temp file recorded this session and drains the
/// registry. Idempotent: a second call after teardown removes nothing. Called
/// at every session exit (headless ask, piped REPL, and TUI teardown).
pub(crate) fn cleanup_session_charts() -> usize {
    let paths = match SESSION_CHART_FILES.lock() {
        Ok(mut files) => std::mem::take(&mut *files),
        Err(_) => Vec::new(),
    };
    cleanup(&paths)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn scratch_dir(label: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("saya-chart-cleanup-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn cleanup_removes_exactly_the_named_paths() {
        let _guard = TEST_LOCK.lock().unwrap();
        let dir = scratch_dir("paths");
        let a = dir.join("a.html");
        let b = dir.join("b.html");
        std::fs::write(&a, "<html></html>").unwrap();
        std::fs::write(&b, "<html></html>").unwrap();
        assert_eq!(
            cleanup(&[a.clone(), b.clone(), dir.join("missing.html")]),
            2
        );
        assert!(!a.exists() && !b.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn session_cleanup_removes_every_recorded_file_and_drains() {
        let _guard = TEST_LOCK.lock().unwrap();
        let dir = scratch_dir("session");
        let a = dir.join("a.html");
        let b = dir.join("b.html");
        std::fs::write(&a, "<html></html>").unwrap();
        std::fs::write(&b, "<html></html>").unwrap();
        record_temp_chart(&a);
        record_temp_chart(&b);
        assert_eq!(cleanup_session_charts(), 2);
        assert!(!a.exists() && !b.exists());
        // Drained: a second teardown is a no-op.
        assert_eq!(cleanup_session_charts(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unrecorded_files_are_left_alone() {
        let _guard = TEST_LOCK.lock().unwrap();
        let dir = scratch_dir("unrecorded");
        let tracked = dir.join("tracked.html");
        let untracked = dir.join("untracked.html");
        std::fs::write(&tracked, "<html></html>").unwrap();
        std::fs::write(&untracked, "<html></html>").unwrap();
        record_temp_chart(&tracked);
        cleanup_session_charts();
        assert!(!tracked.exists());
        assert!(
            untracked.exists(),
            "a file the session never wrote must survive teardown"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn automatic_chart_files_are_unique_and_cleanup_does_not_touch_explicit_files() {
        let _guard = TEST_LOCK.lock().unwrap();
        let first = create_temp_chart().expect("first automatic chart");
        let second = create_temp_chart().expect("second automatic chart");
        assert_ne!(first, second);
        assert!(
            first
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("saya-chart-") && name.ends_with(".html"))
        );
        #[cfg(unix)]
        assert_eq!(
            std::fs::metadata(&first).unwrap().permissions().mode() & 0o777,
            0o600
        );

        let explicit = std::env::temp_dir().join(format!(
            "saya-chart-explicit-{}-{}.html",
            std::process::id(),
            first.file_name().unwrap().to_string_lossy()
        ));
        std::fs::write(&explicit, "explicit").unwrap();
        cleanup_session_charts();
        assert!(!first.exists() && !second.exists());
        assert!(explicit.exists(), "explicit chart files are user-owned");
        let _ = std::fs::remove_file(explicit);
    }

    #[cfg(unix)]
    #[test]
    fn automatic_chart_does_not_follow_the_old_predictable_symlink() {
        let _guard = TEST_LOCK.lock().unwrap();
        use std::os::unix::fs::symlink;

        let temp = std::env::temp_dir();
        let old = temp.join("saya-chart.html");
        let target = temp.join(format!("saya-chart-target-{}.html", std::process::id()));
        let _ = std::fs::remove_file(&old);
        std::fs::write(&target, "sentinel").unwrap();
        symlink(&target, &old).unwrap();

        let generated = create_temp_chart().expect("automatic chart");
        std::fs::write(&generated, "chart").unwrap();
        cleanup_session_charts();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "sentinel");
        assert!(
            old.exists(),
            "an explicit/unrecorded path must survive cleanup"
        );
        let _ = std::fs::remove_file(old);
        let _ = std::fs::remove_file(target);
    }
}
