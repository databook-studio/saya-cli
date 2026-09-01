//! Spec D — the `Decide` write path: resolve a short on-screen reference to
//! exactly one claim, then reach the **existing** confirm/reject/use-once
//! operations. Split from `contracts_write.rs` by concern: the resolution
//! step (prefix → one claim id, or a typed refusal) is this slice's only new
//! logic; the operations themselves are unchanged.
//!
//! Confirm and reject emit `ContractChanged` exactly as `review` does (the
//! same ops, the same event). `UseOnce` writes nothing — `use_candidate_once`
//! is request-scoped — so it emits an honest one-line message that names the
//! claim and says it stays a candidate, NOT a `ContractChanged` (which would
//! imply a durable write this operation does not make).
//!
//! `profile_name` is the human-facing name (renders); `identity` is what the
//! store resolves the prefix against. Both come from `resolve_profile`, which
//! keeps the opaque identity out of every message.

use super::{ArgMessage, arg_failure, op_failure};
use crate::cli::ReviewDecisionArg;
use crate::commands::output::emit;
use crate::contracts::{confirm, reject, resolve_prefix, use_candidate_once};
use crate::render::{RenderFormat, TerminalEvent};
use saya_store::SqliteStateStore;
use saya_types::{ClaimStatus, ProfileIdentity};

/// The short reference must be at least the `c-` marker: a bare `c` or empty
/// string matches every claim id (all start with `c-`) and is refused as
/// ambiguous at the resolve step, but rejecting the degenerate prefix here
/// gives a clearer usage error than a store round trip would.
const MIN_PREFIX_LEN: usize = 2;

/// Resolves `prefix` to one claim of `identity`'s profile and applies `decision`
/// through the existing operations. A prefix that matches zero or more than
/// one claim is a payload-free usage error that changes nothing.
pub(super) async fn decide(
    store: &SqliteStateStore,
    format: RenderFormat,
    prefix: &str,
    decision: ReviewDecisionArg,
    profile_name: &str,
    identity: &ProfileIdentity,
) -> Result<i32, Box<dyn std::error::Error>> {
    // A degenerate prefix (empty or just `c`) would match every claim; refuse
    // it as ambiguous here for a clearer message than the resolve step's.
    if prefix.len() < MIN_PREFIX_LEN {
        return arg_failure(ArgMessage::AmbiguousPrefix, format);
    }
    let id = match resolve_prefix(store, identity, prefix).await {
        Ok(id) => id,
        Err(error) => return prefix_failure(error, format),
    };
    match decision {
        ReviewDecisionArg::Confirm => {
            let claim = match confirm(store, &id).await {
                Ok(claim) => claim,
                Err(error) => return op_failure(error, format),
            };
            emit(
                TerminalEvent::ContractChanged {
                    claim_id: claim.id.as_str().to_string(),
                    action: "confirmed".into(),
                    status: "confirmed".into(),
                },
                format,
            );
            Ok(0)
        }
        ReviewDecisionArg::Reject => {
            let claim = match reject(store, &id).await {
                Ok(claim) => claim,
                Err(error) => return op_failure(error, format),
            };
            let (action, status) = match claim.status {
                ClaimStatus::Rejected => ("rejected", "rejected"),
                other => ("reviewed", other.as_str()),
            };
            emit(
                TerminalEvent::ContractChanged {
                    claim_id: claim.id.as_str().to_string(),
                    action: action.into(),
                    status: status.into(),
                },
                format,
            );
            Ok(0)
        }
        ReviewDecisionArg::UseOnce => {
            // `use_candidate_once` validates the claim is a live `Candidate` and
            // writes nothing — a candidate stays a candidate. The
            // admission's effect (one candidate reaching the next recall's
            // `admit_candidate`) is request-scoped and lives on the recall
            // request of the *interactive* turn that follows; threading it from
            // the session loop is a separate, larger change (see the report's
            // "what I did not do"). This path reaches the op — the spec D
            // deliverable for use-once — and reports only what is durably true
            // on every path: the claim is a candidate and nothing was written.
            // It must NOT read as a durable change (`ContractChanged` would
            // overstate it) and must NOT claim an admission it cannot honour
            // here (the "supplied never applied" correctness constraint).
            if let Err(error) = use_candidate_once(store, &id).await {
                return op_failure(error, format);
            }
            // Names the claim, says it stays a candidate. `profile_name` is the
            // human-facing name, never the opaque identity. `Result` (not
            // `ContractChanged`) because this operation writes nothing.
            emit(
                TerminalEvent::Result {
                    message: format!(
                        "{id} is a live candidate; use-once does not confirm it (nothing written, profile: {profile_name})"
                    ),
                },
                format,
            );
            Ok(0)
        }
    }
}

/// Maps a prefix-resolution failure to a payload-free message. `Conflict` is
/// ambiguity (more than one match); `NotFound` is no match. Neither echoes the
/// typed prefix. Any other op error degrades through `op_failure`.
fn prefix_failure(
    error: crate::contracts::ContractOpError,
    format: RenderFormat,
) -> Result<i32, Box<dyn std::error::Error>> {
    match error {
        crate::contracts::ContractOpError::Conflict => {
            arg_failure(ArgMessage::AmbiguousPrefix, format)
        }
        crate::contracts::ContractOpError::NotFound => {
            arg_failure(ArgMessage::PrefixNotFound, format)
        }
        other => op_failure(other, format),
    }
}
