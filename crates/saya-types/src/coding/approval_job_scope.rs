use serde::{Deserialize, Serialize};

/// The job-level operation a human is being asked to approve.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum JobApprovalAction {
    Start,
    Resume,
}
