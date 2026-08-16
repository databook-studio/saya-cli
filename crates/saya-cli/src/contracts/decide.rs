//! Spec D — resolve a short on-screen reference to exactly one claim.
//!
//! The short reference is a leading prefix of a stored [`ClaimId`] — the
//! `c-xxxx` form `contracts list`/`show` already abbreviate to
//! (`render_contract::abbreviate_id`). This module only *resolves* the prefix
//! to a single claim id; the confirm/reject/use-once operations themselves
//! live in [`super::review`] and [`super::use_once`] and are **not**
//! reimplemented here. A confirm in particular still revalidates against the
//! live (cached) schema exactly as `contracts review --confirm` does.
//!
//! Why a prefix and not a per-turn index: a per-turn index recycles every turn,
//! so a user who reads a receipt, runs another query, then types `/confirm 2`
//! silently confirms a *different* claim — a wrong write to durable memory,
//! the one thing this feature exists not to do. A stored id prefix is an
//! immutable primary key; it never recycles. The failure mode is not "points
//! at the wrong claim" but "does not resolve" — zero matches, or more than
//! one. Both refuse and change nothing. So a stale reference can never
//! silently write the wrong claim: it resolves to the same claim, or refuses.
//!
//! There is no store-level prefix scan: [`ContractStore::get_claim`] is an
//! exact `WHERE id = ?`, and [`ContractStore::list_claims`] needs a
//! [`DatabaseObjectRef`]. The only whole-profile read is
//! [`ContractStore::list_claims_for_profile`], so the prefix is matched
//! in Rust against every claim of the resolved profile. The claim set per
//! profile is small (a worklist, not an archive), so this is bounded.

use super::op_error::ContractOpError;

use saya_store::{ContractStore, SqliteStateStore};
use saya_types::{ClaimId, ProfileIdentity};

/// Resolves `prefix` to exactly one claim id among `profile`'s claims, or a
/// typed refusal. Zero matches → [`ContractOpError::NotFound`]; two or more →
/// [`ContractOpError::Conflict`] (the "unambiguous or refused" invariant). The
/// prefix must be non-empty and a plausible id prefix (start with `c-`); a
/// prefix that could not match any id is `NotFound`, not a panic.
///
/// `Conflict` reads as "ambiguous" at the adapter: more than one claim shares
/// the prefix, so the user must type more characters. The message is
/// payload-free — the typed prefixes themselves never reach it (see
/// `commands::ArgMessage::AmbiguousPrefix`).
pub(crate) async fn resolve_prefix(
    store: &SqliteStateStore,
    profile: &ProfileIdentity,
    prefix: &str,
) -> Result<ClaimId, ContractOpError> {
    let claims = store.list_claims_for_profile(profile).await?;
    let matching: Vec<ClaimId> = claims
        .into_iter()
        .map(|c| c.id)
        .filter(|id| id.as_str().starts_with(prefix))
        .collect();
    match matching.len() {
        0 => Err(ContractOpError::NotFound),
        1 => Ok(matching.into_iter().next().expect("len == 1")),
        // More than one claim shares the prefix: ambiguous. Refuse rather
        // than guess which one the user meant — acting on the wrong claim
        // silently is worse than refusing (the binding invariant).
        _ => Err(ContractOpError::Conflict),
    }
}
