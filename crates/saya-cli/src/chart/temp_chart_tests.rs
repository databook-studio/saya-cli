//! Regression tests for the reservation/output replacement window (A006):
//! a symlink swapped in after the automatic path is reserved must not
//! redirect the chart HTML write.

use super::super::cleanup::{cleanup_session_charts, record_temp_chart};
use super::{OpenOptions, PathBuf, TempChart, lock_charts_for_test, reserve_temp_chart};

fn scratch_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("saya-temp-chart-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn reserve_scratch_chart(dir: &std::path::Path) -> TempChart {
    let path = dir.join("reserved.html");
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .unwrap();
    TempChart { file, path }
}

#[test]
fn write_goes_to_the_reserved_file_not_the_reopened_path() {
    let _guard = lock_charts_for_test();
    let dir = scratch_dir("descriptor");
    let mut chart = reserve_scratch_chart(&dir);
    chart.write_html("chart-body").unwrap();
    assert_eq!(std::fs::read_to_string(chart.path()).unwrap(), "chart-body");
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn write_ignores_a_symlink_swapped_in_after_reservation() {
    use std::os::unix::fs::symlink;

    let _guard = lock_charts_for_test();
    let dir = scratch_dir("symlink-window");
    let target = dir.join("outside.html");
    std::fs::write(&target, "sentinel").unwrap();

    // Reserve the chart, then simulate the window: an actor replaces the
    // reserved path with a symlink to an outside file before output.
    let mut chart = reserve_scratch_chart(&dir);
    std::fs::remove_file(chart.path()).unwrap();
    symlink(&target, chart.path()).unwrap();

    // The write must still land in the reserved file, not follow the link.
    chart.write_html("chart-body").unwrap();

    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "sentinel",
        "replacing the reserved path between reservation and output must not redirect the write"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn reserved_charts_are_unique_private_and_recorded_for_teardown() {
    let _guard = lock_charts_for_test();
    // Drain charts reserved by other tests so teardown counts below are exact.
    cleanup_session_charts();
    let first = reserve_temp_chart().expect("first automatic chart");
    let second = reserve_temp_chart().expect("second automatic chart");
    assert_ne!(first.path(), second.path());
    assert!(
        first
            .path()
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("saya-chart-") && name.ends_with(".html"))
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(first.path())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }

    // Cleanup still removes reserved files; record the scratch-file
    // contract explicitly via the shared registry seam.
    let dir = scratch_dir("teardown");
    let scratch = dir.join("scratch.html");
    std::fs::write(&scratch, "scratch").unwrap();
    record_temp_chart(&scratch);
    let removed = cleanup_session_charts();
    assert!(removed >= 3, "reserved and scratch files are all torn down");
    assert!(!first.path().exists() && !second.path().exists());
    assert!(!scratch.exists());
    let _ = std::fs::remove_dir_all(&dir);
}
