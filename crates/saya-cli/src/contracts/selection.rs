//! Recall selection: which objects match a request, and in what order.
//!
//! Order (plan §11.1), lower tier wins, ties broken by most-recently-seen then
//! object name for determinism:
//!  1. an exact `explicit_ref`;
//!  2. an exact confirmed `TableAlias` match (lowercased, trimmed) within the
//!     active profiles;
//!  3. a bounded lexical match on the qualified name or confirmed description
//!     text;
//!  4. most-recently-seen in the active profiles — tie-breaker only, never the
//!     sole reason an object is selected.
//!
//! No cross-profile fallback: a term that matches nothing in the active profiles
//! matches nothing. Ambiguity returns every match.

use crate::contracts::availability::SchemaAvailability;
use saya_store::{ContractStore, SqliteStateStore, StoredClaim, StoredObject};
use saya_types::{ClaimPayload, DatabaseObjectRef, ProfileIdentity};

/// One object's recallable claims plus the live schema for its profile and the
/// `last_seen` stamp used only as a tie-breaker.
pub(crate) struct Candidate {
    pub object: DatabaseObjectRef,
    pub claims: Vec<StoredClaim>,
    pub last_seen_unix_ms: i64,
    pub tier: u8,
}

pub(crate) struct Selection {
    pub candidates: Vec<Candidate>,
    pub considered: usize,
    pub excluded_by_status: usize,
}

/// Builds the ranked candidate list for `request` against `store`. Returns the
/// selection untouched by bounds or privacy — the caller applies those.
pub(crate) async fn select(
    store: &SqliteStateStore,
    request: &super::RecallRequest<'_>,
    live_schemas: &[(ProfileIdentity, SchemaAvailability)],
) -> Result<Selection, saya_store::StoreError> {
    let active: Vec<StoredObject> = collect_objects(store, request.profiles).await?;
    let considered = active.len();

    let mut by_object: Vec<Candidate> = Vec::new();
    let mut excluded_by_status = 0usize;
    for obj in &active {
        let all = store.list_claims(&obj.object, &[]).await?;
        let recallable: Vec<StoredClaim> = all
            .into_iter()
            .filter(|c| request.recall_mode.admits(c.status))
            .collect();
        if recallable.is_empty() {
            // The object has claims but none are admitted by this mode — under
            // `Confirmed` every one was a candidate/rejected/stale/contradicted/
            // forgotten; under `IncludeCandidates` it had none of confirmed or
            // candidate.
            excluded_by_status += 1;
            continue;
        }
        let tier = best_tier(&obj.object, &recallable, request);
        by_object.push(Candidate {
            object: obj.object.clone(),
            claims: recallable,
            last_seen_unix_ms: obj.last_seen_unix_ms,
            tier,
        });
    }

    // Rank: tier asc, then most-recently-seen desc, then object name asc (deterministic).
    by_object.sort_by(|a, b| {
        a.tier
            .cmp(&b.tier)
            .then(b.last_seen_unix_ms.cmp(&a.last_seen_unix_ms))
            .then(a.object.qualified_name().cmp(&b.object.qualified_name()))
    });

    // Keep only objects that the request actually asks for. Tier 4 (recency
    // alone) is a tie-breaker, never sole evidence — drop it unless another tier
    // selected it. With no terms and no explicit refs, nothing is selected.
    let has_query = !request.explicit_refs.is_empty() || !request.terms.is_empty();
    if has_query {
        by_object.retain(|c| c.tier < 4);
    } else {
        by_object.clear();
    }

    let _ = live_schemas;
    Ok(Selection {
        candidates: by_object,
        considered,
        excluded_by_status,
    })
}

async fn collect_objects(
    store: &SqliteStateStore,
    profiles: &[ProfileIdentity],
) -> Result<Vec<StoredObject>, saya_store::StoreError> {
    let mut out = Vec::new();
    for profile in profiles {
        out.extend(store.list_objects(profile).await?);
    }
    Ok(out)
}

fn best_tier(
    object: &DatabaseObjectRef,
    claims: &[StoredClaim],
    request: &super::RecallRequest<'_>,
) -> u8 {
    if request.explicit_refs.iter().any(|r| r == object) {
        return 1;
    }
    let aliases: Vec<String> = claims
        .iter()
        .filter_map(|c| match c.payload.as_ref()? {
            ClaimPayload::TableAlias { alias, .. } => Some(alias.trim().to_lowercase()),
            _ => None,
        })
        .collect();
    let term_lc: Vec<String> = request
        .terms
        .iter()
        .map(|t| t.trim().to_lowercase())
        .collect();
    if term_lc.iter().any(|t| aliases.iter().any(|a| a == t)) {
        return 2;
    }
    let qn = object.qualified_name().to_lowercase();
    let desc_text: Vec<String> = claims
        .iter()
        .filter_map(|c| match c.payload.as_ref()? {
            ClaimPayload::TableDescription { text, .. } => Some(text.to_lowercase()),
            ClaimPayload::TableGrain { description, .. } => Some(description.to_lowercase()),
            ClaimPayload::ColumnDescription { text, .. } => Some(text.to_lowercase()),
            _ => None,
        })
        .collect();
    if term_lc
        .iter()
        .any(|t| qn.contains(t) || desc_text.iter().any(|d| d.contains(t)))
    {
        return 3;
    }
    4
}
