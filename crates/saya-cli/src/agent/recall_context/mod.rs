//! Recall into the agent prompt — slice 2b-3b §3.
//!
//! Glue between [`crate::contracts::recall`] (typed contracts) and
//! [`AgentRequest::context_blocks`] (untrusted, labelled data in the user
//! turn). Called from [`super::runtime::run_prompt_with_sink`] just before the
//! request is built; the wrapper, escaping, and user-turn placement are
//! `saya-agent`'s job (Phase 2a).
//!
//! Decisions recorded in the SPEC REVIEW:
//! - **Stale claims are included, plainly labelled** — excluding would discard
//!   the query-shaping signal this slice exists to deliver.
//! - **Schemas are the store-cached tree** per profile, not a live `connector`
//!   round-trip — recall must not add a connection call on every prompt.
//! - **Identities come from the registry** (profiles that actually connected),
//!   not a re-derivation from `RuntimeConfig`.

mod render;

use crate::connection::ConnectionRegistry;
use crate::contracts::{PromptTerms, RecallBounds, RecallRequest, recall, terms};
use saya_agent::ContextBlock;
use saya_store::{SchemaStore, SqliteStateStore};
use saya_types::{DatabaseObjectRef, ProfileIdentity, SchemaTree};

/// Label every produced block carries. Stable and machine-ish, never localised.
pub(crate) const BLOCK_LABEL: &str = "database-contracts";

/// Builds the context blocks for a prompt from recalled contracts.
///
/// `allow_database_context == false` skips recall entirely — the store is not
/// queried (§3.1: not querying is both cheaper and a stronger guarantee). Zero
/// contracts → no block at all (§3.5). Store failure → no block and no error
/// (§4). `truncated` is true if recall truncated at any bound (§3.4).
pub(crate) async fn recall_context_blocks(
    prompt: &str,
    allow_database_context: bool,
    registry: &ConnectionRegistry,
    state_db: Option<&SqliteStateStore>,
) -> Vec<ContextBlock> {
    // §3.1: skip recall entirely when database context is off. Not querying is
    // both cheaper and a stronger guarantee than querying and discarding.
    if !allow_database_context {
        return Vec::new();
    }
    let Some(store) = state_db else {
        return Vec::new();
    };
    // §4: recall must not run when there is nothing to recall against.
    let Some((identities, schemas)) = resolve_profiles(registry, store).await else {
        return Vec::new();
    };
    if identities.is_empty() || prompt.trim().is_empty() {
        return Vec::new();
    }

    let PromptTerms { explicit, terms } = terms::extract(prompt);
    let explicit_refs = build_refs(&identities, &explicit);
    // §4: no explicit refs and no terms → selection matches nothing. Don't
    // ask the store to confirm that.
    if explicit_refs.is_empty() && terms.is_empty() {
        return Vec::new();
    }

    let request = RecallRequest {
        profiles: &identities,
        explicit_refs: &explicit_refs,
        terms: &terms,
        allow_database_context: true,
        schemas: &schemas,
        bounds: RecallBounds::defaults(),
    };
    let outcome = recall(store, request).await;
    // §4: store failure or nothing selected → no block, no error.
    if outcome.diagnostics.store_unavailable || outcome.contracts.is_empty() {
        return Vec::new();
    }

    let name_of = render::name_by_identity(registry);
    let truncated = outcome.contracts.iter().any(|c| c.truncated);
    let body = render::render_body(&outcome.contracts, &name_of);
    if body.is_empty() {
        return Vec::new();
    }
    vec![ContextBlock {
        label: BLOCK_LABEL.to_string(),
        body,
        truncated,
    }]
}

/// Resolves the connected profiles to identities plus their cached schemas.
/// Returns `None` when no profile carries an identity (e.g. a test registry).
async fn resolve_profiles(
    registry: &ConnectionRegistry,
    store: &SqliteStateStore,
) -> Option<(Vec<ProfileIdentity>, Vec<(ProfileIdentity, SchemaTree)>)> {
    let mut identities = Vec::new();
    let mut schemas = Vec::new();
    for (_name, entry) in registry.entries() {
        let Some(id_str) = entry.profile_id.as_deref() else {
            continue;
        };
        let Ok(identity) = ProfileIdentity::parse(id_str) else {
            continue;
        };
        // Store-cached schema, not a live connector round-trip (SPEC REVIEW): a
        // missing cache degrades to an empty tree → `live_schema_unavailable`,
        // never a fabricated real one.
        let schema = store
            .get_schema(identity.as_str())
            .await
            .ok()
            .flatten()
            .map(|cached| cached.schema)
            .unwrap_or_default();
        identities.push(identity.clone());
        schemas.push((identity, schema));
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
fn build_refs(
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

#[cfg(test)]
#[path = "../recall_context_tests.rs"]
mod tests;
