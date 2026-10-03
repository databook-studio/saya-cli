//! Typed progress for a provider attempt that did not produce a usable reply.

use serde::{Deserialize, Serialize};

/// The recovery disposition of a provider attempt. These events report
/// progress; the existing completion or error outcome still settles the run.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ProviderRecoveryPhase {
    /// The receiver will retry the unchanged request after its existing delay.
    Retrying,
    /// The receiver exhausted its fixed retry allowance.
    Exhausted,
    /// This failure is not eligible for a receive retry.
    NotRetried,
}

/// A locally established reason a provider attempt did not yield a reply.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ProviderRecoveryReason {
    EmptyResponse,
    StreamEnded,
    StreamByteLimit,
    ToolCollectionLimit,
    OutputTruncated,
    ProviderFailure,
    Cancelled,
}
