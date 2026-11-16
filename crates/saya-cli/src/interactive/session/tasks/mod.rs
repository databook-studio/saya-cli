//! `checklist`: whole-list replace of the session task list.
//!
//! The one session-metadata write the model reaches for: one idempotent call
//! carrying the **whole list**, not per-item add/complete/delete — a
//! whole-list replace has no delete-versus-complete ambiguity and no
//! partial-update race, and it is one schema instead of four.
//!
//! The payload is untrusted model input, so it validates through
//! [`SessionTaskList::validate`] before storing — the `RunPlan::validate`
//! discipline. A rejected write stores nothing: the stored list is replaced
//! only by a list that passed. Validation failures are typed, payload-free,
//! and name the bound breached so the model can correct itself.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use saya_agent::{ToolError, ToolExecutor};
use saya_types::{SessionTaskError, SessionTaskList};

/// The session's live task list: the in-memory working copy the `checklist`
/// executor reads and writes, together with the watermark that decides
/// whether a request still has to carry it. A plain mutex-guarded cell — a
/// panicked holder cannot have left it in a state recovery must fear, so a
/// poisoned lock keeps serving (the engine's own rule for its grant store).
#[derive(Debug, Clone, Default)]
pub(crate) struct SessionTasks(Arc<Mutex<TasksCell>>);

/// The stored list and the two facts the render gate reads: the revision a
/// change bumps, and the revision the last request carried.
#[derive(Debug, Default)]
struct TasksCell {
    list: SessionTaskList,
    /// Bumped by a replace that actually changed the stored list.
    revision: u64,
    /// The revision the last request carried. `None` until one has.
    rendered: Option<u64>,
}

impl SessionTasks {
    /// The current list, cloned out — what the record sync writes back and
    /// what the operator's own views read.
    pub(crate) fn current(&self) -> SessionTaskList {
        self.locked().list.clone()
    }

    /// The list this request must carry, or `None` when it has not changed
    /// since the last request carried it — and always for an empty list.
    /// Taking it records that this revision has been rendered.
    pub(crate) fn take_changed(&self) -> Option<SessionTaskList> {
        let mut cell = self.locked();
        if cell.list.is_empty() || cell.rendered == Some(cell.revision) {
            return None;
        }
        cell.rendered = Some(cell.revision);
        Some(cell.list.clone())
    }

    /// Replaces the stored list with one the caller already validated — the
    /// executor validates before calling, so this never sees a bad list. A
    /// replace that stores an equal list is not a change: the session record
    /// re-seeds the same list at the start of every turn, and re-rendering it
    /// would re-bill the block for nothing.
    pub(crate) fn replace(&self, list: SessionTaskList) {
        let mut cell = self.locked();
        if cell.list != list {
            cell.list = list;
            cell.revision += 1;
        }
    }

    fn locked(&self) -> MutexGuard<'_, TasksCell> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The `checklist` executor member: whole-list replace against the shared
/// cell. Runs through [`ToolExecutor`] so the session's composite can route
/// the name to it like every other member.
pub(crate) struct TasksSet {
    tasks: SessionTasks,
}

impl TasksSet {
    pub(crate) fn new(tasks: SessionTasks) -> Self {
        Self { tasks }
    }

    /// The render gate's read side: what the next request carries from this
    /// list, or `None` when it has not changed since the last one carried it.
    pub(crate) fn take_changed(&self) -> Option<SessionTaskList> {
        self.tasks.take_changed()
    }

    /// Validates the call arguments' shape: one object with exactly the
    /// `tasks` array. Typed errors name which argument is wrong — the model
    /// fixes the right one — and the rejected payload is never echoed.
    fn validate_arguments(arguments: &serde_json::Value) -> Result<(), ToolError> {
        let object = arguments.as_object().ok_or(ToolError::ArgumentsNotObject)?;
        if object.keys().any(|key| key != "tasks") {
            return Err(ToolError::UnsupportedProperty);
        }
        if !object.get("tasks").is_some_and(serde_json::Value::is_array) {
            return Err(ToolError::TasksNotArray);
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl ToolExecutor for TasksSet {
    async fn execute(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        if name != "checklist" {
            return Err(ToolError::UnsupportedTool);
        }
        Self::validate_arguments(&arguments)?;
        // Deserialize first — a non-deserializing payload is a shape error,
        // not a bound error — then validate through the contract's own gate
        // before storing anything. The whole arguments object deserializes:
        // the contract's list shape is `{"tasks": [...]}`, exactly this
        // tool's schema.
        let list: SessionTaskList =
            serde_json::from_value(arguments).map_err(|_| ToolError::TasksNotArray)?;
        list.validate().map_err(|error| match error {
            SessionTaskError::TooManyTasks(_) => ToolError::TooManyTasks,
            SessionTaskError::TooManyInProgress(_) => ToolError::TooManyInProgress,
            SessionTaskError::EmptyTitle => ToolError::TaskTitleEmpty,
            SessionTaskError::TitleTooLong => ToolError::TaskTitleTooLong,
            SessionTaskError::TitleControlCharacter => ToolError::TaskTitleControl,
            SessionTaskError::NoteTooLong => ToolError::TaskNoteTooLong,
            SessionTaskError::NoteControlCharacter => ToolError::TaskNoteControl,
            // `SessionTaskError` is `#[non_exhaustive]`: a future variant
            // refuses as a shape error rather than failing to compile — the
            // call still stores nothing.
            _ => ToolError::TasksNotArray,
        })?;
        let count = list.tasks.len();
        self.tasks.replace(list);
        Ok(serde_json::json!({"tasks": count}))
    }
}

pub(crate) mod render;
pub(crate) mod view;

#[cfg(test)]
mod tests;
