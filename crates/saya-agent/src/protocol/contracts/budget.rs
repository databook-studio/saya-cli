//! Fixed answering-budget dimensions and source-owned count observations.

use serde::{Deserialize, Serialize};

/// The answering budget or external bound that stopped further work.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum BudgetDimension {
    LogicalRequests,
    RequestedToolCalls,
    ElapsedTime,
    ReportedTokens,
    DownloadBytes,
    RequestContextBytes,
    RequestContextWindow,
}

/// Authoritative count facts emitted by the answering loop before effects.
///
/// A request admission starts an unresolved tool-call observation. Only a
/// completed response emits `ToolBatchObserved`, including an explicit zero.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum AnsweringObservation {
    Initialized,
    RequestAdmitted,
    ToolBatchObserved { requested: u64 },
}
