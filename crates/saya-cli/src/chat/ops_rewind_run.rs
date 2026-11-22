//! The run behind `/undo` and `/rewind`: its own checkpoint store, read and
//! restored through the one rewind operation the job already owns.
//!
//! This is the only adapter a surface needs to hand the two commands their run;
//! it adds no restore of its own. The words of a turn are the surface's — a
//! checkpoint record holds paths and versions, never a message — so they are
//! left out here rather than guessed.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use super::{Checkpoint, Checkpoints, Restored};
use crate::commands::coding::NativeCodingJob;

/// The run's own checkpoints.
pub(crate) struct RunCheckpoints {
    job: Arc<NativeCodingJob>,
}

impl RunCheckpoints {
    /// The checkpoints of `job`.
    #[allow(dead_code)] // a surface that owns a run's job hands it in; no front runs these two commands yet
    pub(crate) fn new(job: Arc<NativeCodingJob>) -> Self {
        Self { job }
    }
}

#[async_trait]
impl Checkpoints for RunCheckpoints {
    async fn checkpoints(&self) -> Result<Vec<Checkpoint>, String> {
        let listed = self
            .job
            .rewind_store()
            .list()
            .map_err(|error| error.to_string())?;
        Ok(listed
            .into_iter()
            .map(|checkpoint| Checkpoint {
                turn: checkpoint.turn,
                files: checkpoint.entries.len(),
                first_words: None,
            })
            .collect())
    }

    async fn restore(&self, turn: u32) -> Result<Restored, String> {
        let result = self
            .job
            .model_rewind(Some(&turn.to_string()), None)
            .await
            .map_err(|error| error.to_string())?;
        Ok(Restored {
            restored: changed_paths(&result),
            changed_elsewhere: skipped_paths(&result),
        })
    }
}

/// The paths one restore changed, whichever way it changed them: a rewritten
/// file and a file removed to match the checkpoint are both named to the
/// operator.
fn changed_paths(result: &Value) -> Vec<String> {
    let restored = result
        .get("restored_paths")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.get("path").and_then(Value::as_str).map(str::to_owned));
    let removed = result
        .get("removed_paths")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.as_str().map(str::to_owned));
    restored.chain(removed).collect()
}

/// The paths the restore left as they stand because another run changed them
/// since this run last wrote them.
fn skipped_paths(result: &Value) -> Vec<String> {
    result
        .get("foreign_paths")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.as_str().map(str::to_owned))
        .collect()
}

#[cfg(test)]
#[path = "ops_rewind_run_tests.rs"]
mod tests;
