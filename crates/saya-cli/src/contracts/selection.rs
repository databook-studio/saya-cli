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
use std::collections::HashMap;

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
///
/// Two store round trips per profile, not one per object: a single
/// [`ContractStore::list_claims_for_profile`] fetches every claim of a profile,
/// and the objects come from [`ContractStore::list_objects`]. Claims are grouped
/// by object and filtered by recall mode in Rust — `best_tier` scores from the
/// decoded payload text, so selection cannot filter before decoding, and the
/// `excluded_by_status` count depends on the full claim set (an object whose
/// claims are all non-admitted reads the same as one with none).
pub(crate) async fn select(
    store: &SqliteStateStore,
    request: &super::RecallRequest<'_>,
    live_schemas: &[(ProfileIdentity, SchemaAvailability)],
) -> Result<Selection, saya_store::StoreError> {
    let active: Vec<StoredObject> = collect_objects(store, request.profiles).await?;
    let considered = active.len();
    let claims_by_object = collect_claims(store, request.profiles).await?;

    let mut by_object: Vec<Candidate> = Vec::new();
    let mut excluded_by_status = 0usize;
    for obj in &active {
        let all = claims_by_object.get(&obj.object);
        let recallable: Vec<StoredClaim> = match all {
            Some(claims) => claims
                .iter()
                .filter(|c| request.recall_mode.admits(c.status))
                .cloned()
                .collect(),
            None => Vec::new(),
        };
        if recallable.is_empty() {
            // The object has claims but none are admitted by this mode — under
            // `Confirmed` every one was a candidate/rejected/stale/contradicted/
            // forgotten; under `IncludeCandidates` it had none of confirmed or
            // candidate. An object with no claims at all lands here too: the
            // count matches the per-object loop it replaces.
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

/// Every claim of every active profile, grouped by object — one
/// `list_claims_for_profile` per profile rather than one `list_claims` per
/// object. The grouping key is the decoded [`DatabaseObjectRef`], the same value
/// `StoredObject.object` carries, so the object loop looks up its claims without
/// re-deriving the store's object id.
async fn collect_claims(
    store: &SqliteStateStore,
    profiles: &[ProfileIdentity],
) -> Result<HashMap<DatabaseObjectRef, Vec<StoredClaim>>, saya_store::StoreError> {
    let mut by_object: HashMap<DatabaseObjectRef, Vec<StoredClaim>> = HashMap::new();
    for profile in profiles {
        for claim in store.list_claims_for_profile(profile).await? {
            by_object
                .entry(claim.object.clone())
                .or_default()
                .push(claim);
        }
    }
    Ok(by_object)
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
    // Tier 3 matches the term against the object's NAME SEGMENT (not the whole
    // qualified name) and confirmed description text. Matching on the name
    // segment alone — rather than `qn.contains(t)` on the full
    // `catalog.schema.object` — stops a term like `public` selecting every table
    // in the `public` schema and a short term like `cat` matching the catalog.
    // Substring containment stays (term ⊆ name segment) so a term still selects a
    // numbered object it prefixes (`orders` → `orders0`); it is now bounded to
    // the object name, never the catalog/schema segments.
    let name_segment = object.object().to_lowercase();
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
        .any(|t| name_matches(&name_segment, t) || desc_text.iter().any(|d| d.contains(t)))
    {
        return 3;
    }
    4
}

/// Whether prompt term `term` (already lowercased, trimmed) matches an object's
/// lowercased name segment at tier 3. A term matches when the segment equals it,
/// equals its singularized form (so a plural prompt term `rentals` names the
/// singular table `rental`), or contains it as a substring (so `orders` still
/// selects `orders0`). Containment is term ⊆ segment, never the reverse: a long
/// term must not match a short table (spec §3 — bidirectional containment is a
/// trap).
///
/// `singular_key` is deliberately small and not a stemmer; see it for the rule
/// and the cases it leaves alone. Irregular plurals (`children`, `people`,
/// `data`) are unsupported by design — naming the limit beats a dependency.
fn name_matches(name_segment: &str, term: &str) -> bool {
    if name_segment == term {
        return true;
    }
    let singular = singular_key(term);
    singular != term && name_segment == singular || name_segment.contains(term)
}

/// The conservative singular form of `term`, or `term` itself when no rule
/// applies. Rules, in order, for a word ending in `s`:
/// - `ies → y` (`categories → category`), stem ≥ 4 so `series` is left alone;
/// - `ses/xes/zes/ches/shes → drop es` (`addresses → address`, `boxes → box`);
/// - a bare trailing `s → drop`, but not for words ending in `ss` (`address`),
///   `us` (`status`), `is` (`axis`), or `ies` (owned by the rule above).
///
/// Deliberately unsupported: irregular plurals (`children`, `people`, `data`)
/// and anything a real stemmer would catch. A word ending in `s` that is already
/// singular is returned unchanged, so the identity match still holds.
fn singular_key(term: &str) -> String {
    let bytes = term.as_bytes();
    let n = bytes.len();
    // `ies → y`, but only when the stem is at least 4 chars so short words like
    // `series` (stem `ser`) are left as-is.
    if n > 3 && term.ends_with("ies") {
        return format!("{}y", &term[..n - 3]);
    }
    // `…ses/xes/zes/ches/shes → drop es`, leaving `s/x/z/ch/sh`.
    if n > 2 && term.ends_with("es") {
        let stem = &term[..n - 2];
        if stem.ends_with(['s', 'x', 'z']) || stem.ends_with("ch") || stem.ends_with("sh") {
            return stem.to_string();
        }
    }
    // Bare trailing `s → drop`, guarded so already-singular `-s` words stay.
    if n > 3
        && term.ends_with('s')
        && !term.ends_with("ss")
        && !term.ends_with("us")
        && !term.ends_with("is")
        && !term.ends_with("ies")
    {
        return term[..n - 1].to_string();
    }
    term.to_string()
}
