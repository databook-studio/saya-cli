//! Resolves the profiles recall runs against.
//!
//! The connected registry entries become the identities and store-cached
//! schema availability `recall` selects against, and the prompt's explicit
//! `@catalog.schema.object` refs become the `DatabaseObjectRef`s bound to
//! every active identity.

use crate::connection::ConnectionRegistry;
use crate::contracts::SchemaAvailability;
use saya_store::{SchemaStore, SqliteStateStore};
use saya_types::{DatabaseObjectRef, ProfileIdentity};

/// Resolves the connected profiles to identities plus their schema
/// availability. Returns `None` when no profile carries an identity (e.g. a
/// test registry).
///
/// The three-way distinction is the P1 fix: a store error (`Unavailable`), a
/// missing cache entry (`Missing`), and a real cache (`Available`) are kept
/// apart. Collapsing the first two into an empty `SchemaTree` — the old
/// behaviour — made validity see a schema that exists but lacks the object and
/// classify every claim `Stale`, so a store hiccup silently muted the model's
/// whole memory and blamed drift. `Missing` and `Unavailable` both classify
/// `LiveSchemaUnavailable`; the freshness bound (applied in `recall` for the
/// model path) treats a too-old `Available` the same way.
pub(super) async fn resolve_profiles(
    registry: &ConnectionRegistry,
    store: &SqliteStateStore,
) -> Option<(
    Vec<ProfileIdentity>,
    Vec<(ProfileIdentity, SchemaAvailability)>,
)> {
    let mut identities = Vec::new();
    let mut schemas = Vec::new();
    for (_name, entry) in registry.entries() {
        let Some(id_str) = entry.profile_id.as_deref() else {
            continue;
        };
        let Ok(identity) = ProfileIdentity::parse(id_str) else {
            continue;
        };
        // Store-cached schema, not a live connector round-trip (SPEC REVIEW):
        // a missing cache and a store error are *not* an empty tree — they are
        // "cannot classify", kept distinct so a diagnostic can name which.
        let availability = match store.get_schema(identity.as_str()).await {
            Ok(Some(cached)) => {
                SchemaAvailability::available(cached.schema, cached.updated_unix_ms)
            }
            Ok(None) => SchemaAvailability::Missing,
            Err(_) => SchemaAvailability::Unavailable,
        };
        identities.push(identity.clone());
        schemas.push((identity, availability));
    }
    if identities.is_empty() {
        None
    } else {
        Some((identities, schemas))
    }
}

/// Builds `DatabaseObjectRef`s for the explicit `@catalog.schema.object` refs,
/// attaching each to every active identity — recall's cross-profile isolation
/// keeps a ref bound to the profile that owns the object.
pub(super) fn build_refs(
    identities: &[ProfileIdentity],
    explicit: &[crate::contracts::args::QualifiedName],
) -> Vec<DatabaseObjectRef> {
    use saya_types::DatabaseObjectKind;
    let mut out = Vec::new();
    for id in identities {
        for q in explicit {
            if let Ok(obj) = DatabaseObjectRef::new(
                id.clone(),
                &q.catalog,
                &q.schema,
                &q.object,
                DatabaseObjectKind::Table,
            ) {
                out.push(obj);
            }
        }
    }
    out
}
