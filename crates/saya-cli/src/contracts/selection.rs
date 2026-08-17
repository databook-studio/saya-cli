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
//!
//! Reads the D-3 `knowledge_items` table via [`KnowledgeItemStore`] — the same
//! rows the harness-owned learning path writes — so what the harness learns is
//! what recall supplies. Two store round trips per profile, not one per object:
//! [`KnowledgeItemStore::knowledge_for_profile`] fetches every item of a
//! profile, and [`KnowledgeItemStore::objects_for_profile`] the distinct
//! objects. Items are grouped by object and filtered by recall mode in Rust —
//! `best_tier` scores from the decoded payload text, so selection cannot
//! filter before decoding, and the `excluded_by_status` count depends on the
//! full item set (an object whose items are all non-admitted reads the same as
//! one with none).

use super::name_match::name_matches;
use crate::contracts::availability::SchemaAvailability;
use saya_store::{KnowledgeItem, KnowledgeItemStore, SqliteStateStore};
use saya_types::{ClaimPayload, DatabaseObjectRef, KnowledgeState, ProfileIdentity};
use std::collections::HashMap;

/// One object's recallable items plus the live schema for its profile and the
/// `last_seen` stamp used only as a tie-breaker. Carries the raw
/// [`KnowledgeItem`]s so assembly can compute validity from each item's
/// `schema_binding_json` + `fingerprint_version` before projecting to the
/// render carrier.
pub(crate) struct Candidate {
    pub object: DatabaseObjectRef,
    pub items: Vec<KnowledgeItem>,
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
) -> Result<Selection, saya_store::KnowledgeStoreError> {
    let active: Vec<DatabaseObjectRef> = collect_objects(store, request.profiles).await?;
    let considered = active.len();
    let items_by_object = collect_items(store, request.profiles).await?;

    let mut by_object: Vec<Candidate> = Vec::new();
    let mut excluded_by_status = 0usize;
    for obj in &active {
        let all = items_by_object.get(obj);
        let recallable: Vec<KnowledgeItem> = match all {
            Some(items) => items
                .iter()
                .filter(|it| {
                    request.recall_mode.admits_state(it.state) || admissible_once(it, request)
                })
                .cloned()
                .collect(),
            None => Vec::new(),
        };
        if recallable.is_empty() {
            // The object has items but none are admitted by this mode — under
            // `Confirmed` every one was pending/dismissed; under
            // `IncludeCandidates` it had no active or pending. An object with
            // no items at all lands here too: the count matches the per-object
            // loop it replaces.
            excluded_by_status += 1;
            continue;
        }
        // `last_seen` is not a column on `knowledge_items`; the tie-breaker
        // uses the most recent write among the object's admitted items.
        let last_seen_unix_ms = all
            .map(|items| items.iter().map(|i| i.updated_unix_ms).max().unwrap_or(0))
            .unwrap_or(0);
        let tier = best_tier(obj, &recallable, request);
        by_object.push(Candidate {
            object: obj.clone(),
            items: recallable,
            last_seen_unix_ms,
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

/// Whether `item` is the single candidate `use_candidate_once` admitted to
/// this recall — the per-item exception alongside `recall_mode.admits_state`.
/// Honoured only for a live `Pending` item: a non-pending id in the request is a
/// no-op, because `use_candidate_once` refuses to mint one for anything else.
fn admissible_once(item: &saya_store::KnowledgeItem, request: &super::RecallRequest<'_>) -> bool {
    request.admit_candidate.as_ref().map(|id| id.as_str()) == Some(&item.id)
        && item.state == KnowledgeState::Pending
}

async fn collect_objects(
    store: &SqliteStateStore,
    profiles: &[ProfileIdentity],
) -> Result<Vec<DatabaseObjectRef>, saya_store::KnowledgeStoreError> {
    let mut out = Vec::new();
    for profile in profiles {
        out.extend(store.objects_for_profile(profile).await?);
    }
    Ok(out)
}

/// Every knowledge item of every active profile, grouped by object — one
/// `knowledge_for_profile` per profile rather than one read per object. The
/// grouping key is the inlined [`DatabaseObjectRef`] the row carries, so the
/// object loop looks up its items without re-deriving an id.
async fn collect_items(
    store: &SqliteStateStore,
    profiles: &[ProfileIdentity],
) -> Result<
    HashMap<DatabaseObjectRef, Vec<saya_store::KnowledgeItem>>,
    saya_store::KnowledgeStoreError,
> {
    let mut by_object: HashMap<DatabaseObjectRef, Vec<saya_store::KnowledgeItem>> = HashMap::new();
    for profile in profiles {
        for item in store.knowledge_for_profile(profile).await? {
            by_object.entry(item.object.clone()).or_default().push(item);
        }
    }
    Ok(by_object)
}

fn best_tier(
    object: &DatabaseObjectRef,
    items: &[KnowledgeItem],
    request: &super::RecallRequest<'_>,
) -> u8 {
    if request.explicit_refs.iter().any(|r| r == object) {
        return 1;
    }
    let aliases: Vec<String> = items
        .iter()
        .filter_map(|it| match &it.value {
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
    // Tier 3 matches the term against the object's NAME SEGMENT (not the whole
    // qualified name) and confirmed description text. Matching on the name
    // segment alone — rather than `qn.contains(t)` on the full
    // `catalog.schema.object` — stops a term like `public` selecting every table
    // in the `public` schema and a short term like `cat` matching the catalog.
    // Substring containment stays (term ⊆ name segment) so a term still selects a
    // numbered object it prefixes (`orders` → `orders0`); it is now bounded to
    // the object name, never the catalog/schema segments.
    let name_segment = object.object().to_lowercase();
    let desc_text: Vec<String> = items
        .iter()
        .filter_map(|it| match &it.value {
            ClaimPayload::TableDescription { text, .. } => Some(text.to_lowercase()),
            ClaimPayload::TableGrain { description, .. } => Some(description.to_lowercase()),
            ClaimPayload::ColumnDescription { text, .. } => Some(text.to_lowercase()),
            _ => None,
        })
        .collect();
    if term_lc
        .iter()
        .any(|t| name_matches(&name_segment, t) || desc_text.iter().any(|d| d.contains(t)))
    {
        return 3;
    }
    4
}
