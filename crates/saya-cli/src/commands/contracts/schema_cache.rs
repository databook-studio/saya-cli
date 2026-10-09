//! The cached-schema conventions every adapter that reads a claim against
//! the store shares: the "no schema observed" sentinel fingerprint, the raw
//! cached tree, and the three-state availability classification.

use crate::contracts::SchemaAvailability;
use saya_store::{SchemaStore, SqliteStateStore};
use saya_types::{FINGERPRINT_VERSION, ProfileIdentity, SchemaFingerprint, SchemaTree};

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

/// The cached schema for `identity`, or `None` when nothing is cached. Mirrors
/// the agent recall path's `SchemaStore::get_schema` lookup, but without the
/// `.unwrap_or_default()` that collapses "no cache" onto an empty tree: a
/// missing cache stays `None` so validity reads the honest
/// `live_schema_unavailable`, while a cached-but-empty tree reads `stale`. A
/// store read failure degrades to `None` — not a crash on a read path.
///
/// Shared by the read commands (`list`/`show`), the write command (`remember`),
/// and the agent contract tools (`contract_search`), so every adapter that
/// classifies a claim against the cached schema does it the same way — never a
/// second lookup convention.
pub(crate) async fn cached_schema(
    store: &SqliteStateStore,
    identity: &ProfileIdentity,
) -> Option<SchemaTree> {
    store
        .get_schema(identity.as_str())
        .await
        .ok()
        .flatten()
        .map(|cached| cached.schema)
}

/// The schema known for a profile as a three-state [`SchemaAvailability`]: a
/// real cache (`Available`, carrying when it was observed), no cache entry yet
/// (`Missing`), or a store that could not be read (`Unavailable`). The read
/// commands and the agent contract tools use this — *not* [`cached_schema`] —
/// so a store error or an undiscovered profile classifies `LiveSchemaUnavailable`
/// instead of collapsing to an empty tree that would read `Stale` (the P1 bug).
/// The write path (`remember`/`import`) keeps [`cached_schema`]: it resolves a
/// fingerprint against whatever the cache holds, which is not a classification.
pub(crate) async fn cached_schema_availability(
    store: &SqliteStateStore,
    identity: &ProfileIdentity,
) -> SchemaAvailability {
    match store.get_schema(identity.as_str()).await {
        Ok(Some(cached)) => SchemaAvailability::available(cached.schema, cached.updated_unix_ms),
        Ok(None) => SchemaAvailability::Missing,
        Err(_) => SchemaAvailability::Unavailable,
    }
}
