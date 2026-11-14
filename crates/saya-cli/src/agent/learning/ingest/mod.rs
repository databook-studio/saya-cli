//! Anti-self-reinforcement filtering and store ingestion.
//!
//! Enforces Safety Property 4 (drops redundant inferences on already supplied claims)
//! and persists verified proposals to [`saya_store::KnowledgeItemStore`].

use saya_store::KnowledgeStoreError;
use saya_types::ContractError;
use thiserror::Error;

/// Errors occurring during proposal ingestion.
#[derive(Debug, Error)]
#[allow(dead_code)]
pub enum IngestionError {
    #[error("failed to serialize schema binding: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("store error: {0}")]
    Store(#[from] KnowledgeStoreError),
    #[error("invalid claim id: {0}")]
    ClaimId(#[from] ContractError),
}

mod filtering;
mod persistence;

pub(crate) use filtering::{filter_anti_self_reinforcement, filter_anti_self_reinforcement_dto};
pub(crate) use persistence::ingest_proposals;

#[cfg(test)]
#[path = "tests/mod.rs"]
mod tests;
