//! Spec D — resolve a short on-screen reference to exactly one knowledge item.
//!
//! The short reference is a leading prefix of a stored item id — the `ki-xxxx`
//! form the receipt/list/show stanzas abbreviate to
//! (`render_contract::abbreviate_id`). This module only *resolves* the prefix
//! to a single id; the confirm/reject/use-once operations themselves live in
//! [`super::review`] and [`super::use_once`] and are **not** reimplemented
//! here. A confirm in particular still revalidates against the live (cached)
//! schema exactly as `contracts review --confirm` does.
//!
//! Why a prefix and not a per-turn index: a per-turn index recycles every turn,
//! so a user who reads a receipt, runs another query, then types `/confirm 2`
//! silently confirms a *different* claim — a wrong write to durable memory,
//! the one thing this feature exists not to do. A stored id prefix is an
//! immutable primary key; it never recycles. The failure mode is not "points
//! at the wrong item" but "does not resolve" — zero matches, or more than
//! one. Both refuse and change nothing. So a stale reference can never
//! silently write the wrong item: it resolves to the same one, or refuses.
//!
//! There is no store-level prefix scan: [`KnowledgeItemStore::get_knowledge_item`]
//! is an exact `WHERE id = ?`. The only whole-profile read is
//! [`KnowledgeItemStore::knowledge_for_profile`], so the prefix is matched in
//! Rust against every item of the resolved profile. The item set per profile is
//! small (a worklist, not an archive), so this is bounded.
//!
//! Chunk 3 moved the decision ops onto `knowledge_items`; the ids the receipt
//! prints are now `ki-…`, so a resolver that only matched the legacy `c-…` ids
//! would silently fail to resolve anything a user can see — the defect this
//! module exists to close.

use super::op_error::ContractOpError;

use saya_store::{KnowledgeItemStore, SqliteStateStore};
use saya_types::{ClaimId, ProfileIdentity};

/// Resolves `prefix` to exactly one item id among `profile`'s knowledge items,
/// or a typed refusal. Zero matches → [`ContractOpError::NotFound`]; two or
/// more → [`ContractOpError::Conflict`] (the "unambiguous or refused"
/// invariant). A prefix that could not match any id is `NotFound`, not a
/// panic — the empty/non-`ki-` cases fall out of that naturally, so the
/// adapter's length guard is a courtesy message, not a correctness check.
///
/// `Conflict` reads as "ambiguous" at the adapter: more than one item shares
/// the prefix, so the user must type more characters. The message is
/// payload-free — the typed prefixes themselves never reach it (see
/// `commands::ArgMessage::AmbiguousPrefix`).
pub(crate) async fn resolve_prefix(
    store: &SqliteStateStore,
    profile: &ProfileIdentity,
    prefix: &str,
) -> Result<ClaimId, ContractOpError> {
    let items = store.knowledge_for_profile(profile).await?;
    let matching: Vec<ClaimId> = items
        .into_iter()
        .filter_map(|item| ClaimId::parse(&item.id).ok())
        .filter(|id| id.as_str().starts_with(prefix))
        .collect();
    match matching.len() {
        0 => Err(ContractOpError::NotFound),
        1 => Ok(matching.into_iter().next().expect("len == 1")),
        // More than one item shares the prefix: ambiguous. Refuse rather
        // than guess which one the user meant — acting on the wrong item
        // silently is worse than refusing (the binding invariant).
        _ => Err(ContractOpError::Conflict),
    }
}
