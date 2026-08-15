//! Bulk read counterparts to the per-object/per-claim reads in [`store_reads`].
//!
//! Recall, the review queue, and reconciliation used to loop over
//! [`crate::contracts::store_reads::list_objects`] and issue one
//! [`crate::contracts::store_reads::list_claims`] per object (and one
//! [`crate::contracts::store_reads::evidence_count`] per queued claim). That was
//! an N+1 of SQLite round trips on the path of every question. The functions
//! here fetch a whole profile's claims, and a whole queue's evidence counts, in
//! one query each — the CLI groups and filters the result in Rust, so the
//! admission rule and the diagnostics counts stay where they were.

use crate::contracts::records::{MAX_LISTED_OBJECTS, StoredClaim};
use crate::contracts::store_decode::{ClaimRow, decode_claim};
use crate::{SqliteStateStore, StoreError};
use saya_types::{ClaimId, ProfileIdentity};

/// The most claim ids one [`evidence_counts`] round trip binds. SQLite caps
/// bound parameters (`SQLITE_MAX_VARIABLE_NUMBER`, 32766 on the bundled build);
/// this stays well under it so an `IN (?, …)` over a chunk never exceeds the
/// limit. The realistic queue admits a handful of candidates, so one chunk is
/// one query.
const EVIDENCE_COUNT_BATCH: usize = 1000;

/// Every claim of `profile` in one query — the bulk counterpart to
/// [`crate::contracts::store_reads::list_claims`], which fetches a single
/// object. Recall and the review queue both need every object's claims for a
/// profile; asking the store once per object was an N+1 on the path of every
/// question. The CLI groups and filters the result in Rust, so this returns
/// every status (no `statuses` filter): the recall-mode admission rule and the
/// `excluded_by_status` count both live in the CLI and depend on the full claim
/// set, and filtering here would silently change that count for an object whose
/// claims are all non-admitted.
///
/// Bounded to the same [`MAX_LISTED_OBJECTS`] most-recently-seen objects
/// [`crate::contracts::store_reads::list_objects`] returns: both call sites used
/// to loop over `list_objects`, so the object cap was already in force. A claim
/// on an object beyond that cap was never seen by recall or the queue, and still
/// is not — the point is fewer round trips, not a larger result.
///
/// Ordered `o.last_seen_unix_ms DESC, o.id ASC, c.created_unix_ms ASC, c.id ASC`
/// — the same sequence the per-object loop produced (`list_objects` then
/// `list_claims` per object). Reconciliation examines claims in that order to
/// decide which fall inside the 1000-claim bound, so the bulk fetch must hand
/// them back in it; a global `created-ASC` order would mark a different 1000 at
/// the bound. Recall and the queue group or re-sort, so the order is free for
/// them but binding here.
pub(crate) async fn list_claims_for_profile(
    store: &SqliteStateStore,
    profile: &ProfileIdentity,
) -> Result<Vec<StoredClaim>, StoreError> {
    let rows = sqlx::query_as::<_, ClaimRow>("SELECT c.id, c.payload_json, c.origin, c.status, c.schema_fingerprint, c.referenced_columns_json, c.created_unix_ms, c.updated_unix_ms, c.last_verified_unix_ms, o.profile_id, o.catalog_name, o.schema_name, o.object_name, o.object_kind, o.fingerprint_version FROM contract_claims c JOIN contract_objects o ON o.id = c.object_id WHERE o.id IN (SELECT id FROM contract_objects WHERE profile_id=? ORDER BY last_seen_unix_ms DESC, id ASC LIMIT ?) ORDER BY o.last_seen_unix_ms DESC, o.id ASC, c.created_unix_ms ASC, c.id ASC")
        .bind(profile.as_str())
        .bind(MAX_LISTED_OBJECTS as i64)
        .fetch_all(store.pool().await?)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    rows.into_iter().map(decode_claim).collect()
}

/// Evidence counts for a set of claims in one query — the bulk counterpart to
/// [`crate::contracts::store_reads::evidence_count`]. The review queue asked for
/// one count per claim it returned; aggregating here with a
/// `LEFT JOIN ... GROUP BY` replaces N `COUNT(*)` round trips with one. The
/// `LEFT JOIN` yields a row per claim, so a claim with no evidence reads `0`
/// (the same value the per-claim [`crate::contracts::store_reads::evidence_count`]
/// returns for a claim that exists with no rows). Unlike the per-claim count,
/// there is no `EXISTS` guard: every `id` came from a `list_claims` call, so they
/// are known to exist, and a bare `0` is the honest answer rather than
/// `NotFound`. Returns `(ClaimId, count)` pairs.
///
/// `ids` is chunked under [`EVIDENCE_COUNT_BATCH`]: SQLite caps bound
/// parameters (`SQLITE_MAX_VARIABLE_NUMBER`), so a single `IN (?, …)` over the
/// whole admitted set — up to one object's worth of claims times the object cap
/// — could exceed the limit. Each chunk is one round trip, so the realistic case
/// (a handful of candidates) is still one query.
pub(crate) async fn evidence_counts(
    store: &SqliteStateStore,
    ids: &[ClaimId],
) -> Result<Vec<(ClaimId, usize)>, StoreError> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut out: Vec<(ClaimId, usize)> = Vec::with_capacity(ids.len());
    for chunk in ids.chunks(EVIDENCE_COUNT_BATCH) {
        let placeholders = vec!["?"; chunk.len()].join(",");
        let sql = format!(
            "SELECT c.id, COUNT(e.id) FROM contract_claims c LEFT JOIN contract_evidence e ON e.claim_id = c.id WHERE c.id IN ({placeholders}) GROUP BY c.id"
        );
        let mut query = sqlx::query_as::<_, (String, i64)>(&sql);
        for id in chunk {
            query = query.bind(id.as_str());
        }
        let rows = query
            .fetch_all(store.pool().await?)
            .await
            .map_err(|_| StoreError::Unavailable)?;
        let parsed: Vec<(ClaimId, usize)> = rows
            .into_iter()
            .map(|(id, count)| {
                Ok((
                    ClaimId::parse(&id).map_err(|_| StoreError::Invalid)?,
                    count.max(0) as usize,
                ))
            })
            .collect::<Result<_, _>>()?;
        out.extend(parsed);
    }
    Ok(out)
}
