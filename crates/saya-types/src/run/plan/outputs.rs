use serde::{Deserialize, Serialize};

use super::is_artifact_name;
use crate::run::RunContractError;

/// How many artifacts one step may declare as expected outputs.
pub const MAX_OUTPUT_HINTS: usize = 16;

const MAX_HINT_DESCRIPTION_CHARS: usize = 512;

/// An artifact a step is expected to produce, named relative to the run
/// workspace. The name is a single path component because it becomes one;
/// the description is bounded free text for the approval view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct OutputHint {
    pub name: String,
    pub description: Option<String>,
}

impl OutputHint {
    pub fn new(
        name: impl Into<String>,
        description: Option<&str>,
    ) -> Result<Self, RunContractError> {
        let name = name.into();
        if !is_artifact_name(&name) {
            return Err(RunContractError::InvalidOutputHint);
        }
        let description = match description.map(str::trim).filter(|d| !d.is_empty()) {
            Some(text) => {
                if text.chars().count() > MAX_HINT_DESCRIPTION_CHARS
                    || text.chars().any(char::is_control)
                {
                    return Err(RunContractError::InvalidOutputHint);
                }
                Some(text.to_string())
            }
            None => None,
        };
        Ok(Self { name, description })
    }
}

/// One deliverable a step declared, resolved against the run workspace at
/// the step's completion. `artifact` is `None` when the declared deliverable
/// was never produced — recorded, never silently dropped: a run may not look
/// complete while a declared output is absent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Deliverable {
    /// The declared name, as the step's hint names it.
    pub name: String,
    /// The artifact as the manifest recorded it — `None` when the step never
    /// produced the deliverable.
    pub artifact: Option<DeliverableArtifact>,
}

impl Deliverable {
    /// A deliverable the step produced: the manifest's size and digest.
    pub fn present(name: impl Into<String>, size: u64, digest: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            artifact: Some(DeliverableArtifact {
                size,
                digest: digest.into(),
            }),
        }
    }

    /// A declared deliverable the step never produced.
    pub fn missing(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            artifact: None,
        }
    }
}

/// The resolved artifact behind a deliverable: its byte size and lowercase
/// hex sha256 — the same manifest discipline the episode brief reports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct DeliverableArtifact {
    pub size: u64,
    /// Lowercase hex sha256 of the file's bytes, as the manifest read them.
    pub digest: String,
}
