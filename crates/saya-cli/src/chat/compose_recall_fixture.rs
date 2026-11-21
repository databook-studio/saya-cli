//! What the composition's recall tests seed: a confirmed claim in the store the
//! session opens, filed under the identity its connection carries.
//!
//! The seeding is the recall-context tests' own — included by path rather than
//! copied, so the row that asserts a chat turn recalls is exercised over
//! exactly the store shape those tests assert against.

use saya_types::ProfileIdentity;

use crate::connection::ConnectionRegistry;

/// The user turn whose terms select the seeded claim: it names the table the
/// claim is filed under, at the name tier recall ranks highest.
pub(super) const QUESTION: &str = "orders by month";

/// The line every recall block opens its body with: the block's label, which
/// the request body carries once recall supplied anything.
pub(super) const RECALL_SOURCE: &str = "source: database-contracts";

/// The profile identity the session's single connection carries — the identity
/// a stored claim is filed under, read back from the registry the session
/// states rather than re-derived from the fixture's profile.
pub(super) fn identity_of(connections: &ConnectionRegistry) -> ProfileIdentity {
    let (name, entry) = connections
        .entries()
        .into_iter()
        .next()
        .expect("the fixture connected one database");
    let identity = entry
        .profile_id
        .as_deref()
        .unwrap_or_else(|| panic!("the fixture connection `{name}` carries a profile identity"));
    ProfileIdentity::parse(identity).expect("the fixture's profile identity is well formed")
}

#[allow(dead_code)] // the fixture carries helpers only some of its callers seed with
pub(super) mod seed {
    use saya_store::{SchemaStore, SqliteStateStore};
    use saya_types::{
        ClaimOrigin, ClaimPayload, Column, Database, DatabaseObjectKind, DatabaseObjectRef,
        KnowledgeSlot, KnowledgeState, ProfileIdentity, Schema, SchemaTree, Table,
    };

    include!("../agent/recall_context_tests/fixtures/schema.rs");

    include!("../agent/recall_context_tests/fixtures/claims.rs");

    /// Seeds the cached `orders` schema and one confirmed
    /// `default_time_column` claim under it, so a later recall reads the claim
    /// as current — the recall-context tests' own acceptance scenario.
    pub(crate) async fn seed_confirmed_claim(store: &SqliteStateStore, identity: &ProfileIdentity) {
        let _ = seed_orders_with_created_at(store, identity).await;
    }
}
