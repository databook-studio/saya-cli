//! The progress handle one turn publishes its checks to.
//!
//! The loop publishes progress after every turn; each check a turn reports is
//! appended to `progress.jsonl` as it happens, so `saya job show` sees the
//! streak while the job runs. The turn keeps the checks it has published, so
//! its end can record them as durable job events.

use saya_agent::{ProgressCheck, ProgressHandle};
use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use crate::commands::coding::{lifecycle::LifecycleTimeline, progress_report};

/// The turn's progress handle, and the checks it publishes.
pub(super) fn handle(
    records: &Path,
    lifecycle: Arc<LifecycleTimeline>,
) -> (ProgressHandle, Arc<Mutex<Vec<ProgressCheck>>>) {
    let checks: Arc<Mutex<Vec<ProgressCheck>>> = Arc::default();
    let observed = Arc::clone(&checks);
    let records = records.to_path_buf();
    let handle = ProgressHandle::new().with_observer(Arc::new(move |snapshot| {
        let Some(check) = snapshot.report.check else {
            return;
        };
        let Ok(mut checks) = observed.lock() else {
            return;
        };
        // A turn boundary can publish twice (the final turn is published by its
        // own end and again with the stop); record it once.
        if checks.last().is_some_and(|last| last.turn == check.turn) {
            return;
        }
        checks.push(check);
        let started = std::time::Instant::now();
        progress_report::record(&records, &snapshot.report);
        lifecycle.note_elapsed("progress_persist", started.elapsed());
    }));
    (handle, checks)
}
