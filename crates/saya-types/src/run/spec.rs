//! The run specification: the [`RunId`] identifying a run and the [`RunSpec`]
//! declaring its goal, its approved scopes, and the budgets it may spend.

use std::fmt;

use serde::{Deserialize, Serialize};

use super::RunContractError;
use super::budget::Budgets;
use super::scope::Capabilities;

/// The longest goal a run or a step may carry, in bytes. A goal is shown to
/// the user at approval time, so it is bounded like every other
/// model-supplied free-text field.
pub const MAX_GOAL_BYTES: usize = 8192;

const MAX_RUN_ID_CHARS: usize = 128;

/// Bounds a goal the way every free-text field the model supplies is bounded:
/// non-empty, within [`MAX_GOAL_BYTES`], and free of control characters — a
/// control character is how untrusted text smuggles structure into a rendered
/// approval view.
pub(crate) fn validate_goal(goal: &str) -> Result<(), RunContractError> {
    if goal.is_empty() {
        return Err(RunContractError::EmptyGoal);
    }
    if goal.len() > MAX_GOAL_BYTES {
        return Err(RunContractError::GoalTooLong);
    }
    if goal.chars().any(char::is_control) {
        return Err(RunContractError::GoalControlCharacter);
    }
    Ok(())
}

/// Opaque, validated run identifier. A run id names the run directory on
/// disk, so its shape is restricted to characters safe to embed in a path
/// component.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct RunId(String);

impl RunId {
    pub fn parse(value: &str) -> Result<Self, RunContractError> {
        if value.is_empty() || value.len() > MAX_RUN_ID_CHARS {
            return Err(RunContractError::InvalidRunId);
        }
        if !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(RunContractError::InvalidRunId);
        }
        Ok(Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RunId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<String> for RunId {
    type Error = RunContractError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

/// What a run is declared with: its goal, the scopes it is approved for, and
/// the budgets it may spend. The endpoint bindings live inside `scopes` —
/// binding roles to endpoints is part of the approval a run asks for, so a
/// step's endpoint must resolve through them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RunSpec {
    pub id: RunId,
    pub goal: String,
    pub scopes: Capabilities,
    pub budgets: Budgets,
}

impl RunSpec {
    /// Validates the goal's shape and the budgets' endpoint keys. A run
    /// arriving through deserialization skipped this constructor; the engine
    /// re-validates a loaded spec the same way it validates a loaded plan.
    pub fn new(
        id: RunId,
        goal: impl Into<String>,
        scopes: Capabilities,
        budgets: Budgets,
    ) -> Result<Self, RunContractError> {
        let goal = goal.into();
        validate_goal(&goal)?;
        budgets.validate()?;
        Ok(Self {
            id,
            goal,
            scopes,
            budgets,
        })
    }
}
