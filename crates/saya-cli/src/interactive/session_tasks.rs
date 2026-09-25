//! `tasks_set`: whole-list replace of the session task list.
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

/// The session's live task list: the in-memory working copy the `tasks_set`
/// executor reads and writes. A plain mutex-guarded cell — a panicked holder
/// cannot have left it in a state recovery must fear, so a poisoned lock
/// keeps serving (the engine's own rule for its grant store).
#[derive(Debug, Clone, Default)]
pub(crate) struct SessionTasks(Arc<Mutex<SessionTaskList>>);

impl SessionTasks {
    /// The current list, cloned out — what a turn's context block renders
    /// and what the record sync writes back.
    pub(crate) fn current(&self) -> SessionTaskList {
        self.locked().clone()
    }

    /// Replaces the stored list with one the caller already validated — the
    /// executor validates before calling, so this never sees a bad list.
    pub(crate) fn replace(&self, list: SessionTaskList) {
        *self.locked() = list;
    }

    fn locked(&self) -> MutexGuard<'_, SessionTaskList> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The `tasks_set` executor member: whole-list replace against the shared
/// cell. Runs through [`ToolExecutor`] so the session's composite can route
/// the name to it like every other member.
pub(crate) struct TasksSet {
    tasks: SessionTasks,
}

impl TasksSet {
    pub(crate) fn new(tasks: SessionTasks) -> Self {
        Self { tasks }
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
        if name != "tasks_set" {
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
