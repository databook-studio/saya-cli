use std::{
    path::{Path, PathBuf},
    process::Child,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

pub(super) struct ChildGuard(pub(super) Option<Child>);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            if child.try_wait().unwrap().is_none() {
                let _ = child.kill();
            }
            let _ = child.wait();
        }
    }
}

pub(super) fn chart_temp_files_in(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with("saya-chart-") && path.extension().is_some_and(|e| e == "html") {
                found.push(path);
            }
        }
    }
    found
}

pub(super) fn clear_chart_temp_files_in(dir: &Path) {
    for path in chart_temp_files_in(dir) {
        let _ = std::fs::remove_file(path);
    }
}

/// A foreign `saya-chart-*.html` planted in the process's real temp directory
/// — the shared directory where another live saya session would legitimately
/// write its own charts. Dropping the guard removes the plant, including when
/// the test panics, so the plant never leaks into the shared directory.
pub(super) struct ForeignChartGuard {
    pub(super) path: PathBuf,
}

impl Drop for ForeignChartGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

pub(super) fn plant_foreign_chart() -> ForeignChartGuard {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.subsec_nanos())
        .unwrap_or(0);
    // "foreign" cannot appear in a production-reserved name
    // (`saya-chart-{16-hex}.html`), so the plant can never collide with a
    // live session's recorded chart.
    let path = std::env::temp_dir().join(format!(
        "saya-chart-foreign-{}-{nonce}.html",
        std::process::id()
    ));
    std::fs::write(&path, "planted by chart_cleanup.rs; must survive the run").unwrap();
    ForeignChartGuard { path }
}

/// Waits for the run to finish (the `complete` event) and then asserts the
/// hidden chart call failed validation — never a denial, never a completion:
/// an unadvertised name is unknown to the loop, so nothing approves it and
/// nothing executes it.
pub(super) fn wait_for_chart_validation_failure(
    buffer: &Arc<Mutex<String>>,
    stderr_text: &Arc<Mutex<String>>,
    timeout: Duration,
) {
    let deadline = Instant::now() + timeout;
    loop {
        let seen = buffer.lock().unwrap().clone();
        if seen
            .lines()
            .any(|line| line.contains("\"event\":\"complete\""))
        {
            drop(seen);
            let done = buffer.lock().unwrap().clone();
            assert!(
                done.lines()
                    .any(|line| line.contains("\"name\":\"render_chart\"")
                        && line.contains("failed validation")),
                "a hidden render_chart must fail validation, never deny or complete; stdout so far:\n{}\nstderr so far:\n{}",
                done,
                stderr_text.lock().unwrap()
            );
            assert!(
                !done
                    .lines()
                    .any(|line| line.contains("\"event\":\"tool_denied\"")
                        && line.contains("render_chart")),
                "a hidden tool is unknown, not denied; stdout so far:\n{}\nstderr so far:\n{}",
                done,
                stderr_text.lock().unwrap()
            );
            return;
        }
        drop(seen);
        assert!(
            Instant::now() < deadline,
            "the run never completed; stdout so far:\n{}\nstderr so far:\n{}",
            buffer.lock().unwrap(),
            stderr_text.lock().unwrap()
        );
        thread::sleep(Duration::from_millis(20));
    }
}
