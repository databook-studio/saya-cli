//! The caller's handle on a run's progress: the latest turn's report and the
//! guard stop, when one fired.

use std::fmt;
use std::sync::{Arc, Mutex};

use super::{ProgressReport, ProgressStop};

/// What the loop publishes for its caller after every turn: the running
/// report, and the guard stop when one fired.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgressSnapshot {
    pub report: ProgressReport,
    pub stop: Option<ProgressStop>,
}

/// A caller's synchronous view of every published turn. The loop calls it at
/// each turn boundary; a caller that records or displays the progress check
/// does so here, and a failure to do either never changes the run.
pub type ProgressObserver = Arc<dyn Fn(&ProgressSnapshot) + Send + Sync>;

/// The caller's handle on a run's progress. A clean progress stop returns
/// `Ok` from the loop (the model writes its final state), so the stop travels
/// here rather than as an error; a hard stop returns `Err` and is also
/// published. `std`'s mutex is enough: the loop updates it between turns, and
/// a reader only needs the latest snapshot.
#[derive(Clone, Default)]
pub struct ProgressHandle {
    inner: Arc<Mutex<Option<ProgressSnapshot>>>,
    observer: Option<ProgressObserver>,
}

impl fmt::Debug for ProgressHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProgressHandle")
            .field("observing", &self.observer.is_some())
            .finish_non_exhaustive()
    }
}

impl ProgressHandle {
    pub fn new() -> Self {
        Self::default()
    }

    /// Observes every turn as it is published, in addition to keeping the
    /// latest snapshot. The observer must not panic: it runs on the loop.
    pub fn with_observer(mut self, observer: ProgressObserver) -> Self {
        self.observer = Some(observer);
        self
    }

    /// Replaces the snapshot with the latest turn's view and hands it to the
    /// observer, when one is attached.
    pub fn publish(&self, report: ProgressReport, stop: Option<ProgressStop>) {
        let snapshot = ProgressSnapshot { report, stop };
        if let Some(observer) = &self.observer {
            observer(&snapshot);
        }
        if let Ok(mut slot) = self.inner.lock() {
            *slot = Some(snapshot);
        }
    }

    /// The last published snapshot, if the loop reached a turn boundary.
    pub fn snapshot(&self) -> Option<ProgressSnapshot> {
        self.inner.lock().ok().and_then(|slot| slot.clone())
    }
}
