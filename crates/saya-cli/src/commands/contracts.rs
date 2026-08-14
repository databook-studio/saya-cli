//! The headless `saya contracts` adapter: resolves arguments, calls the
//! `crate::contracts` operations, maps results to render DTOs, and emits them.
//!
//! No policy lives here. What may be recalled, what conflicts, what is stale,
//! and whether a claim may be stored confirmed are all decided in
//! `crate::contracts` and the store — this module only resolves a profile, builds
//! the typed request, and renders the typed result. 2b-4's slash/agent adapters
//! call the same operations and must not need to duplicate anything here.
//!
//! Dispatch and shared helpers live here; profile resolution is in
//! `contracts_profile.rs`, read commands in `contracts_read.rs`, write commands
//! in `contracts_write.rs`, and view→DTO mapping in `contracts_map.rs`.

mod contracts_map;
mod contracts_profile;
mod contracts_read;
mod contracts_write;

// The identity-dropping `RetrievedContract → ContractView` mapping, re-exported
// `pub(crate)` so the agent contract tools (2b-3a) reuse it instead of carrying
// a second mapping that could leak the opaque profile identity.
pub(crate) use contracts_map::contract_view;
pub(crate) use contracts_map::queue_item_view;
// The identity-dropping profile resolution, re-exported `pub(crate)` so the
// preferences adapter (5c-2) reuses it rather than carrying a second one.
pub(crate) use contracts_profile::resolve_profile;

use super::output::failure_message;
use crate::cli::ContractsCommand;
use crate::config::runtime::RuntimeConfig;
use crate::contracts::ContractOpError;
use crate::render::RenderFormat;
use saya_store::SqliteStateStore;
use saya_types::{ClaimId, FINGERPRINT_VERSION, SchemaFingerprint};

/// Exit code for any typed contract-command failure (usage error, op error, or a
/// write against an unavailable store). Matches the user-error code `config`
/// uses; the scheme here is ad-hoc per command like the rest of the crate.
pub(super) const EXIT_CONTRACT_ERROR: i32 = 2;

pub async fn run_contracts(
    command: ContractsCommand,
    runtime: &RuntimeConfig,
    format: RenderFormat,
    store: &SqliteStateStore,
) -> Result<i32, Box<dyn std::error::Error>> {
    match command {
        ContractsCommand::List { profile } => {
            contracts_read::list(store, runtime, format, profile.as_deref()).await
        }
        ContractsCommand::Show { table, profile } => {
            contracts_read::show(store, runtime, format, &table, profile.as_deref()).await
        }
        ContractsCommand::Queue { profile, limit } => {
            contracts_read::queue(store, runtime, format, profile.as_deref(), limit).await
        }
        ContractsCommand::Remember {
            table,
            kind,
            value,
            column,
            profile,
        } => match resolve_profile(runtime, profile.as_deref()) {
            Ok((_name, identity)) => {
                contracts_write::remember(
                    store,
                    format,
                    &identity,
                    &table,
                    kind,
                    &value,
                    column.as_deref(),
                )
                .await
            }
            Err((code, message)) => failure_message(code, message, format),
        },
        ContractsCommand::Review {
            claim_id,
            confirm,
            reject,
        } => contracts_write::review(store, format, &claim_id, confirm, reject).await,
        ContractsCommand::Forget { claim_id, reason } => {
            contracts_write::forget_claim(store, format, &claim_id, reason).await
        }
    }
}

/// Emits a typed contract-operation error and returns the contract error exit
/// code. `ContractOpError` is payload-free, so no identity or value can leak.
pub(super) fn op_failure(
    error: ContractOpError,
    format: RenderFormat,
) -> Result<i32, Box<dyn std::error::Error>> {
    failure_message(EXIT_CONTRACT_ERROR, error.to_string(), format)
}

/// Emits a payload-free argument-error message and returns the contract error
/// exit code: malformed table, bad value, ambiguous review flags.
pub(super) fn arg_failure(
    message: ArgMessage,
    format: RenderFormat,
) -> Result<i32, Box<dyn std::error::Error>> {
    failure_message(EXIT_CONTRACT_ERROR, message.to_string(), format)
}

/// The "no schema observed" fingerprint: current format, all-zero digest. The
/// headless adapter has no live schema to fingerprint, so it stores a digest
/// guaranteed never to equal a real schema's — a later live schema reads the
/// claim as `needs_review` (or `stale` if a referenced column is gone), never as
/// `current`. Fabricating a real-looking digest would risk a false match.
///
/// `pub(crate)` so the agent contract tools (2b-3a) reuse this same sentinel
/// instead of inventing a second all-zero digest convention.
pub(crate) fn unobserved_fingerprint() -> SchemaFingerprint {
    SchemaFingerprint::from_parts(FINGERPRINT_VERSION, &"0".repeat(64))
        .expect("current format with a 64-hex-zero digest is a valid fingerprint")
}

/// Parses a claim id, mapping a malformed one to a payload-free typed message
/// so an untrusted id never reaches the terminal.
pub(super) fn parse_claim_id(input: &str) -> Result<ClaimId, String> {
    ClaimId::parse(input).map_err(|_| ArgMessage::MalformedClaimId.to_string())
}

/// Payload-free argument-error messages, so untrusted input (a bad table name,
/// a bad claim id, a value the store refused) never reaches the terminal.
#[derive(Debug, Clone, Copy)]
pub(super) enum ArgMessage {
    MalformedTable,
    MalformedClaimId,
    BadValue,
    AmbiguousReview,
}

impl std::fmt::Display for ArgMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MalformedTable => write!(
                f,
                "qualified name must be exactly three dot-separated parts: catalog.schema.object"
            ),
            Self::MalformedClaimId => write!(f, "claim id must be alphanumeric, '-', or '_'"),
            Self::BadValue => write!(f, "claim value is invalid"),
            Self::AmbiguousReview => write!(f, "choose exactly one of --confirm or --reject"),
        }
    }
}
