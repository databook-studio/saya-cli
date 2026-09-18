//! The session task list: conversation metadata tracking what a building
//! session is working on. It binds no authority — unlike a
//! [`RunPlan`](super::RunPlan), it carries no capabilities or budgets — but
//! it is still model-proposed, untrusted input, so [`SessionTaskList::validate`]
//! re-checks every bound itself for lists arriving as JSON, which skip the
//! constructors. The resume path restores a stored list behind that gate.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// How many tasks one session may track: the same order of magnitude as the
/// step and destination bounds the run contracts carry, not a measurement.
pub const MAX_SESSION_TASKS: usize = 32;

/// How long one task title may be, in characters: a title renders as one
/// line in a status view, so it stays a line that still reads as one line.
pub const MAX_TASK_TITLE_CHARS: usize = 200;

/// How long one task note may be, in characters: the output hint's
/// description bound (`run/plan.rs`), the same kind of short annotation.
pub const MAX_TASK_NOTE_CHARS: usize = 512;

/// Why a session task list rejected a value.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum SessionTaskError {
    #[error("task title must not be empty")]
    EmptyTitle,
    #[error("task title is too long")]
    TitleTooLong,
    #[error("task title contains control characters")]
    TitleControlCharacter,
    #[error("task note is too long")]
    NoteTooLong,
    #[error("task note contains control characters")]
    NoteControlCharacter,
    #[error("task list has too many tasks ({0})")]
    TooManyTasks(usize),
    #[error("task list has too many tasks in progress ({0})")]
    TooManyInProgress(usize),
}

/// Where one task stands: waiting, being worked on, or finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum TaskStatus {
    Pending,
    InProgress,
    Done,
}

/// One tracked task: a bounded title, its status, and an optional bounded
/// note. Titles trim at construction so whitespace cannot smuggle an "empty"
/// title past the emptiness check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SessionTask {
    pub title: String,
    pub status: TaskStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl SessionTask {
    pub fn new(title: impl Into<String>, status: TaskStatus) -> Result<Self, SessionTaskError> {
        Self::with_note(title, status, None::<&str>)
    }

    pub fn with_note(
        title: impl Into<String>,
        status: TaskStatus,
        note: Option<impl AsRef<str>>,
    ) -> Result<Self, SessionTaskError> {
        let title = title.into().trim().to_string();
        validate_title(&title)?;
        let note = match note.map(|note| note.as_ref().trim().to_string()) {
            Some(text) if !text.is_empty() => Some(validate_note(&text)?),
            _ => None,
        };
        Ok(Self {
            title,
            status,
            note,
        })
    }
}

/// The ordered tasks a session tracks. Empty is the normal starting state.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
#[non_exhaustive]
pub struct SessionTaskList {
    pub tasks: Vec<SessionTask>,
}

impl SessionTaskList {
    pub fn new(tasks: Vec<SessionTask>) -> Result<Self, SessionTaskError> {
        let list = Self { tasks };
        list.validate()?;
        Ok(list)
    }

    /// Re-checks every bound for a list that may have arrived as JSON: the
    /// task count, each title and note, and the single in-progress rule — a
    /// list where everything is in progress tracks nothing.
    pub fn validate(&self) -> Result<(), SessionTaskError> {
        if self.tasks.len() > MAX_SESSION_TASKS {
            return Err(SessionTaskError::TooManyTasks(self.tasks.len()));
        }
        for task in &self.tasks {
            validate_title(&task.title)?;
            if let Some(note) = &task.note {
                validate_note(note)?;
            }
        }
        let in_progress = self
            .tasks
            .iter()
            .filter(|t| t.status == TaskStatus::InProgress)
            .count();
        if in_progress > 1 {
            return Err(SessionTaskError::TooManyInProgress(in_progress));
        }
        Ok(())
    }

    /// True when the list carries no tasks: the state a record without the
    /// field resumes as.
    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }
}

/// A title is bounded like every model-supplied free-text field: non-empty,
/// within [`MAX_TASK_TITLE_CHARS`], and free of control characters — the
/// rule `validate_name` applies to object names, not a second spelling.
fn validate_title(title: &str) -> Result<(), SessionTaskError> {
    if title.trim().is_empty() {
        return Err(SessionTaskError::EmptyTitle);
    }
    if title.chars().count() > MAX_TASK_TITLE_CHARS {
        return Err(SessionTaskError::TitleTooLong);
    }
    if title.chars().any(char::is_control) {
        return Err(SessionTaskError::TitleControlCharacter);
    }
    Ok(())
}

/// A note is bounded like an output hint's description: within
/// [`MAX_TASK_NOTE_CHARS`] and free of control characters, which smuggle
/// structure into a rendered view.
fn validate_note(note: &str) -> Result<String, SessionTaskError> {
    if note.chars().count() > MAX_TASK_NOTE_CHARS {
        return Err(SessionTaskError::NoteTooLong);
    }
    if note.chars().any(char::is_control) {
        return Err(SessionTaskError::NoteControlCharacter);
    }
    Ok(note.to_string())
}
