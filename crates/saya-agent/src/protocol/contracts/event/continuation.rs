//! The provider-continuation disposition of a run that hit the provider's
//! output-token cap. Progress only: the run's existing completion or error
//! outcome still settles it.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ProviderContinuationPhase {
    /// The receiver admitted the continuation; a re-ask follows.
    Continuing,
    /// The continuation allowance ran out; the truncation error settles the run.
    Exhausted,
}
