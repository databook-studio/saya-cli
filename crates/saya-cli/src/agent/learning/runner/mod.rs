//! Post-turn structured extraction execution runner.

use super::{ExtractionError, IngestionError};
use saya_agent::{ProposedClaimDto, ProviderError, TokenUsage};
use std::fmt;

/// Errors that may occur during post-turn extraction.
#[derive(Debug)]
pub(crate) enum ExtractionRunnerError {
    Provider(ProviderError),
    Extraction(ExtractionError),
    Ingestion(IngestionError),
}

impl fmt::Display for ExtractionRunnerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Provider(e) => write!(f, "extraction provider error: {e:?}"),
            Self::Extraction(e) => write!(f, "extraction parse error: {e}"),
            Self::Ingestion(e) => write!(f, "extraction ingestion error: {e}"),
        }
    }
}

impl std::error::Error for ExtractionRunnerError {}

impl From<ProviderError> for ExtractionRunnerError {
    fn from(e: ProviderError) -> Self {
        Self::Provider(e)
    }
}

impl From<ExtractionError> for ExtractionRunnerError {
    fn from(e: ExtractionError) -> Self {
        Self::Extraction(e)
    }
}

impl From<IngestionError> for ExtractionRunnerError {
    fn from(e: IngestionError) -> Self {
        Self::Ingestion(e)
    }
}

/// What a post-turn extraction call produced: the persisted proposals (on
/// success) and the usage the provider reported for the call whenever a
/// response was received. A parse or ingestion failure still carries the
/// usage — tokens may have been billed before the failure — so the accounting
/// is not tied to the success path. A provider error or timeout produces no
/// response, so the usage is `None` and the recorder invents nothing.
pub(crate) struct ExtractionOutcome {
    pub dtos: Result<Vec<ProposedClaimDto>, ExtractionRunnerError>,
    pub usage: Option<TokenUsage>,
}

impl ExtractionOutcome {
    fn ok(dtos: Vec<ProposedClaimDto>, usage: Option<TokenUsage>) -> Self {
        Self {
            dtos: Ok(dtos),
            usage,
        }
    }

    fn failed(error: ExtractionRunnerError, usage: Option<TokenUsage>) -> Self {
        Self {
            dtos: Err(error),
            usage,
        }
    }
}

mod execute;

pub(crate) use execute::run_extraction;

#[cfg(test)]
#[path = "tests/mod.rs"]
mod tests;
