//! Deterministic file and check facts: observations of actual operations,
//! each carried with the tool that observed it. The bound checks live in
//! `validate`.

use serde::{Deserialize, Serialize};

/// One deterministic file fact from an actual operation: the path relative to
/// the workspace root, the content digest when one was taken, when it was
/// observed, and which tool observed it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileFact {
    /// Relative to the workspace root: never absolute, never a `..` escape.
    pub path: String,
    /// The file's sha256 when one was taken: it detects drift between the
    /// recorded path and the file, it does not authenticate the file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    pub observed_unix_ms: i64,
    pub provenance: FactProvenance,
}

/// One deterministic check fact from an actual operation: what ran, how it
/// ended, when, and which tool ran it. A persisted check is historical data —
/// never fresh evidence or authority on resume.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckFact {
    pub label: String,
    pub outcome: CheckOutcome,
    pub observed_unix_ms: i64,
    pub provenance: FactProvenance,
}

/// How a recorded check ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum CheckOutcome {
    Passed,
    Failed,
    TimedOut,
}

/// How a fact came to be recorded. There is deliberately one variant: facts
/// are deterministic observations of actual operations, so only a tool
/// observation counts — model prose can never certify a file or a test.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum FactProvenance {
    ToolObserved {
        /// The observing tool's name, name-shaped like every run-scoped name.
        tool: String,
    },
}
