//! `/undo` and `/rewind`: the run's own checkpoints, as a chat session sees
//! them.
//!
//! One operation, three surfaces: `saya job rewind`, `/rewind` and `/undo` all
//! restore through the job's checkpoint store and the one restore it already
//! owns (`commands::coding::rewind`). Nothing here writes a file itself —
//! [`Checkpoints`] is what the run answers with — so this module holds the
//! wording and the two rules, never a second rewind.
//!
//! The two rules:
//!
//! * **A session op never runs while a turn runs.** Both commands move files
//!   under the turn in flight, so both refuse with one sentence until it ends.
//! * **`/undo` restores the checkpoint taken before the last turn.** The store
//!   records the working checkout at a turn boundary, so the newest checkpoint
//!   holds what the last editing turn left and the one before it is the state
//!   that turn began with.
//!
//! Files only: a checkpoint never holds a command, a download or a database
//! effect, and neither command pretends otherwise.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::commands::coding::NativeCodingJob;
use crate::slash::RewindRequest;

/// What both commands answer while a turn is running: a session op moves files
/// under the turn, so it waits — it is never queued behind it, and never runs
/// inside it.
pub(crate) const TURN_RUNNING: &str =
    "a turn is running: /undo and /rewind restore files, so they wait until it ends";

/// One checkpoint of the run, as the two commands read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Checkpoint {
    /// The checkpoint's number: what `/rewind <id>` names.
    pub(crate) turn: u32,
    /// How many paths the checkpoint holds.
    pub(crate) files: usize,
    /// The first words of the turn the surface can name for this checkpoint:
    /// the operator's own message, cut short. `None` when the surface has no
    /// words for it — a checkpoint is never given words it did not have.
    pub(crate) first_words: Option<String>,
}

/// What `/undo` and `/rewind` ask of the run.
///
/// The run answers with its own checkpoint store, oldest first, and restores
/// through the one rewind operation; a session never reaches a file itself.
#[async_trait]
pub(crate) trait Checkpoints: Send + Sync {
    /// The run's checkpoints, oldest first: the order the rewind operation
    /// lists them in.
    async fn checkpoints(&self) -> Result<Vec<Checkpoint>, String>;

    /// Restores `turn` and answers the workspace-relative paths it wrote back.
    async fn restore(&self, turn: u32) -> Result<Vec<String>, String>;
}

/// Why one of the two commands changed nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// A turn is running: [`TURN_RUNNING`].
    TurnRunning,
    /// No turn of this run has checkpointed anything yet.
    NothingToUndo,
    /// One checkpoint and no earlier one: the state before the run's only
    /// editing turn is the run's baseline, which `/undo` does not state.
    NoEarlierCheckpoint { turn: u32 },
    /// The run holds no checkpoint at all, so there is nothing to restore.
    NoCheckpoint,
    /// The id named no checkpoint of this run.
    UnknownCheckpoint(String),
    /// The run refused the restore, in its own words.
    Refused(String),
}

impl Refusal {
    /// The sentence the operator reads. One wording per refusal, so a listing
    /// and a refused restore cannot describe the same state differently.
    pub(crate) fn sentence(&self) -> String {
        match self {
            Self::TurnRunning => TURN_RUNNING.to_owned(),
            Self::NothingToUndo => {
                "no turn of this run has edited a file yet, so there is nothing to undo".to_owned()
            }
            Self::NoEarlierCheckpoint { turn } => format!(
                "checkpoint {turn} is this run's only one, taken after the turn that edited: \
                 /undo needs the checkpoint before it, and `saya job rewind <job> --baseline` \
                 restores the run's baseline"
            ),
            Self::NoCheckpoint => {
                "this run has taken no checkpoint yet: /rewind has nothing to restore".to_owned()
            }
            Self::UnknownCheckpoint(id) => {
                format!("{id} names no checkpoint of this run: /rewind lists its checkpoints")
            }
            Self::Refused(words) => words.clone(),
        }
    }
}

/// `/undo`: restore the files the last turn edited, and name them.
pub(crate) async fn undo(
    checkpoints: &dyn Checkpoints,
    turn_running: bool,
) -> Result<String, Refusal> {
    let all = list(checkpoints, turn_running).await?;
    let Some(index) = all.len().checked_sub(2) else {
        return Err(match all.last() {
            Some(only) => Refusal::NoEarlierCheckpoint { turn: only.turn },
            None => Refusal::NothingToUndo,
        });
    };
    let target = &all[index];
    let paths = checkpoints
        .restore(target.turn)
        .await
        .map_err(Refusal::Refused)?;
    Ok(restored("undid the last turn's edits", target.turn, &paths))
}

/// `/rewind`: the run's checkpoints newest first, or one of them restored.
pub(crate) async fn rewind(
    checkpoints: &dyn Checkpoints,
    request: &RewindRequest,
    turn_running: bool,
) -> Result<String, Refusal> {
    let all = list(checkpoints, turn_running).await?;
    let id = match request {
        RewindRequest::List => return Ok(listing(&all)),
        RewindRequest::Restore(id) => id,
    };
    if all.is_empty() {
        return Err(Refusal::NoCheckpoint);
    }
    let target = all
        .iter()
        .find(|checkpoint| checkpoint.turn.to_string() == *id)
        .ok_or_else(|| Refusal::UnknownCheckpoint(id.clone()))?;
    let paths = checkpoints
        .restore(target.turn)
        .await
        .map_err(Refusal::Refused)?;
    Ok(restored("restored the checkpoint", target.turn, &paths))
}

/// The run's checkpoints, unless a turn is running: both commands read them
/// from the run, and a turn's own checkpoint is not taken yet while it runs.
async fn list(
    checkpoints: &dyn Checkpoints,
    turn_running: bool,
) -> Result<Vec<Checkpoint>, Refusal> {
    if turn_running {
        return Err(Refusal::TurnRunning);
    }
    checkpoints.checkpoints().await.map_err(Refusal::Refused)
}

/// The listing: every checkpoint, newest first, with what it holds and the
/// words of the turn the surface can name for it.
fn listing(all: &[Checkpoint]) -> String {
    if all.is_empty() {
        return format!(
            "This run has no checkpoints yet.\n{}",
            Refusal::NoCheckpoint.sentence()
        );
    }
    let mut text = String::from("This run's checkpoints, newest first:");
    for checkpoint in all.iter().rev() {
        let files = if checkpoint.files == 1 {
            "1 file".to_owned()
        } else {
            format!("{} files", checkpoint.files)
        };
        let words = match checkpoint.first_words.as_deref() {
            Some(words) => format!(" · \"{words}\""),
            None => String::new(),
        };
        text.push_str(&format!("\n  {} · {files}{words}", checkpoint.turn));
    }
    text.push_str("\n/rewind <id> restores one; /undo undoes the last turn's edits.");
    text
}

/// What a restore answers: the files it put back, by name.
fn restored(lead: &str, turn: u32, paths: &[String]) -> String {
    if paths.is_empty() {
        return format!("{lead}: checkpoint {turn} already holds every file, so nothing changed");
    }
    format!(
        "{lead}: restored {} from checkpoint {turn}",
        paths.join(", ")
    )
}

/// The run's own checkpoints: the store behind `saya job rewind`, read and
/// restored through the run the session drives.
///
/// This is the only adapter a surface needs to hand `/undo` and `/rewind`
/// their run; it adds no restore of its own. The words of a turn are the
/// surface's — a checkpoint record holds paths and versions, never a
/// message — so they are left out here rather than guessed.
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

    async fn restore(&self, turn: u32) -> Result<Vec<String>, String> {
        let result = self
            .job
            .model_rewind(Some(&turn.to_string()), None)
            .await
            .map_err(|error| error.to_string())?;
        Ok(changed_paths(&result))
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

#[cfg(test)]
#[path = "ops_rewind_fixture.rs"]
mod fixture;

#[cfg(test)]
#[path = "ops_rewind_undo_tests.rs"]
mod undo_tests;

#[cfg(test)]
#[path = "ops_rewind_rewind_tests.rs"]
mod rewind_tests;
