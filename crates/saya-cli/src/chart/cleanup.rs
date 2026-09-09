//! Session-scoped lifecycle for chart temp files (DESIGN §6.6).
//!
//! `render_chart` writes `saya-chart-*.html` into the temp directory; without
//! lifecycle management they accumulate forever. Every auto-generated path is
//! recorded at write time and removed when the session ends. Files the user
//! names explicitly (the TUI `/chart <path>` form) are never recorded and are
//! therefore never deleted.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

static SESSION_CHART_FILES: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// Records a chart temp file written this session so the session teardown can
/// remove it. Best-effort: a poisoned registry only means the file outlives
/// the session, exactly as it did before the registry existed.
pub(crate) fn record_temp_chart(path: &Path) {
    if let Ok(mut files) = SESSION_CHART_FILES.lock() {
        files.push(path.to_path_buf());
    }
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

    fn scratch_dir(label: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("saya-chart-cleanup-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn cleanup_removes_exactly_the_named_paths() {
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
}
