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
    use super::super::temp_chart::lock_charts_for_test;
    use super::*;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    fn scratch_dir(label: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("saya-chart-cleanup-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn cleanup_removes_exactly_the_named_paths() {
        let _guard = lock_charts_for_test();
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
        let _guard = lock_charts_for_test();
        let dir = scratch_dir("session");
        let a = dir.join("a.html");
        let b = dir.join("b.html");
        std::fs::write(&a, "<html></html>").unwrap();
        std::fs::write(&b, "<html></html>").unwrap();
        record_temp_chart(&a);
        record_temp_chart(&b);
        let removed = cleanup_session_charts();
        assert!(
            removed >= 2,
            "teardown removes every recorded file (removed {removed})"
        );
        assert!(!a.exists() && !b.exists());
        // Drained: a second teardown is a no-op.
        assert_eq!(cleanup_session_charts(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unrecorded_files_are_left_alone() {
        let _guard = lock_charts_for_test();
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
        use super::super::temp_chart::reserve_temp_chart;

        let _guard = lock_charts_for_test();
        // Drain charts reserved by other tests so teardown counts below are exact.
        cleanup_session_charts();
        let first = reserve_temp_chart().expect("first automatic chart");
        let second = reserve_temp_chart().expect("second automatic chart");
        let (first, second) = (first.path().to_path_buf(), second.path().to_path_buf());
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
    fn automatic_chart_write_ignores_a_symlink_swapped_in_after_reservation() {
        use std::os::unix::fs::symlink;

        use super::super::temp_chart::reserve_temp_chart;

        let _guard = lock_charts_for_test();
        let dir = scratch_dir("reservation-window");
        let target = dir.join("outside.html");
        std::fs::write(&target, "sentinel").unwrap();

        // Reserve an automatic chart, then simulate the window: an actor
        // replaces the reserved path with a symlink to an outside file
        // before the HTML is written. The write must still land in the
        // reserved file because the descriptor is held through output.
        let mut chart = reserve_temp_chart().expect("automatic chart");
        std::fs::remove_file(chart.path()).unwrap();
        symlink(&target, chart.path()).unwrap();
        chart.write_html("chart-body").unwrap();

        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            "sentinel",
            "replacing the reserved path between reservation and output must not redirect the write"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn automatic_chart_does_not_follow_the_old_predictable_symlink() {
        use super::super::temp_chart::reserve_temp_chart;

        let _guard = lock_charts_for_test();
        use std::os::unix::fs::symlink;

        let temp = std::env::temp_dir();
        let old = temp.join("saya-chart.html");
        let target = temp.join(format!("saya-chart-target-{}.html", std::process::id()));
        let _ = std::fs::remove_file(&old);
        std::fs::write(&target, "sentinel").unwrap();
        symlink(&target, &old).unwrap();

        let mut generated = reserve_temp_chart().expect("automatic chart");
        generated.write_html("chart").unwrap();
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
