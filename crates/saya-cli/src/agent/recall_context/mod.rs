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

mod budget;
mod dispute;
mod receipt;
mod render;

// The single source of truth for the short rendered value a claim shows. The
// prompt body and the recall receipt read it inside this module; the propose
// tool reuses it so a `KnowledgeProposed` event names the same value a later
// recall would, never a divergent one (spec P2d).
pub(crate) use render::claim_value;

use crate::connection::ConnectionRegistry;
use crate::contracts::{
    PromptTerms, RecallBounds, RecallMode, RecallOutcomeKind, RecallReceipt, RecallRequest,
    RetrievalPolicy, SchemaAvailability, recall, terms,
};
use saya_agent::ContextBlock;
use saya_store::{SchemaStore, SqliteStateStore};
use saya_types::{DatabaseObjectRef, ProfileIdentity};

/// Label every produced block carries. Stable and machine-ish, never localised.
pub(crate) const BLOCK_LABEL: &str = "database-contracts";

/// Builds the context blocks for a prompt from recalled contracts, and a
/// [`RecallReceipt`] naming exactly which claims reached the block (spec P1a).
///
/// `allow_database_context == false` skips recall entirely — the store is not
/// queried (§3.1: not querying is both cheaper and a stronger guarantee) and the
/// receipt's [`RecallOutcomeKind`] is `Skipped`. Zero contracts → no block at
/// all (§3.5) and an empty `Ran` receipt. Store failure → no block and no error
/// (§4) and an empty `Ran { store_unavailable: true }` receipt. `truncated` is
/// true if recall truncated at any bound (§3.4); the receipt's `dropped_by_bounds`
/// counts the claims the bounds dropped so a subset never reads as the whole.
///
/// `recall_mode` selects which claim statuses reach the block: `Confirmed`
/// (today's behaviour) or `IncludeCandidates` (candidates admitted and
/// plainly labelled as unconfirmed by the render layer). `bounds` replace the
/// hard-coded `RecallBounds::defaults()`; the caller reads them from config.
///
/// `system_prompt` is the extra system context the runtime has already assembled
/// (connection descriptions, the last-SQL hint). It is `None` when there is none.
/// The byte bound is enforced against the *rendered* block the request actually
/// sends — not the serialized payloads `recall` selected — and against what is
/// left of the agent message budget after the system prompt and the user's own
/// question. The question is the point and the context is the assist, so context
/// never consumes the bytes the prompt needs: [`bound_body`] drops contracts from
/// the end (least-relevant first) until the rendered block fits, and if even the
/// first contract does not fit it is omitted and the block is marked truncated.
pub(crate) async fn recall_context_blocks(
    prompt: &str,
    system_prompt: Option<&str>,
    allow_database_context: bool,
    recall_mode: RecallMode,
    bounds: RecallBounds,
    registry: &ConnectionRegistry,
    state_db: Option<&SqliteStateStore>,
) -> (Vec<ContextBlock>, RecallReceipt) {
    // §3.1: skip recall entirely when database context is off. Not querying is
    // both cheaper and a stronger guarantee than querying and discarding. The
    // receipt marks this as Skipped (policy), distinct from Ran-and-found-nothing.
    if !allow_database_context {
        return (Vec::new(), RecallReceipt::skipped());
    }
    let Some(store) = state_db else {
        return (Vec::new(), RecallReceipt::ran_empty(false));
    };
    // §4: recall must not run when there is nothing to recall against.
    let Some((identities, schemas)) = resolve_profiles(registry, store).await else {
        return (Vec::new(), RecallReceipt::ran_empty(false));
    };
    if identities.is_empty() || prompt.trim().is_empty() {
        return (Vec::new(), RecallReceipt::ran_empty(false));
    }

    let PromptTerms { explicit, terms } = terms::extract(prompt);
    let explicit_refs = build_refs(&identities, &explicit);
    // §4: no explicit refs and no terms → selection matches nothing. Don't
    // ask the store to confirm that.
    if explicit_refs.is_empty() && terms.is_empty() {
        return (Vec::new(), RecallReceipt::ran_empty(false));
    }

    let request = RecallRequest {
        profiles: &identities,
        explicit_refs: &explicit_refs,
        terms: &terms,
        allow_database_context: true,
        schemas: &schemas,
        now_unix_ms: crate::contracts::now_unix_ms(),
        bounds,
        recall_mode,
        // This block is shown to the model, so a contract computed `Stale` is
        // dropped (and counted) rather than read as a current fact. The
        // human-facing `contracts list` is the path that keeps stale.
        policy: RetrievalPolicy::ForModel,
    };
    let outcome = recall(store, request).await;
    // §4: store failure → no block, no error; the receipt records the failure
    // (Ran, store unavailable) so a later phase can name it without an error.
    if outcome.diagnostics.store_unavailable {
        return (Vec::new(), RecallReceipt::ran_empty(true));
    }
    // §4: nothing selected → no block, no error; an empty Ran receipt.
    if outcome.contracts.is_empty() {
        return (Vec::new(), RecallReceipt::ran_empty(false));
    }

    let name_of = render::name_by_identity(registry);
    // `truncated` is true if a *count* bound (objects or claims-per-object) cut
    // a contract short — the byte bound below sets its own flag when it drops
    // contracts to fit the rendered budget.
    let count_truncated = outcome.contracts.iter().any(|c| c.truncated);
    let (body, byte_truncated, kept) = budget::bound_body(
        &outcome.contracts,
        &name_of,
        system_prompt,
        prompt,
        bounds.max_bytes,
    );
    // Claims the bounds dropped: the count-bound drops `recall` already counted
    // in `excluded_by_count_bounds`, plus the whole contracts the byte bound
    // dropped from the end (the tail beyond `kept`).
    let byte_dropped_claims: usize = outcome.contracts[kept..]
        .iter()
        .map(|c| c.claims.len())
        .sum();
    let dropped_by_bounds = outcome.diagnostics.excluded_by_count_bounds + byte_dropped_claims;
    let receipt = RecallReceipt {
        kind: RecallOutcomeKind::Ran {
            store_unavailable: false,
        },
        supplied: receipt::supplied_contracts(&outcome.contracts[..kept], &name_of),
        dropped_by_bounds,
    };
    // `bound_body` returns an empty body only when even the first contract's
    // rendered stanza does not fit the budget — the oversized-first-claim case the
    // old code let through. Omit every claim but still surface a truncated block
    // so the model learns recall happened and the context was too large to
    // include, rather than reading silence as "nothing was remembered".
    if body.is_empty() {
        return (
            vec![ContextBlock {
                label: BLOCK_LABEL.to_string(),
                body,
                truncated: true,
            }],
            // The byte bound dropped every selected claim: `kept == 0`, so
            // `supplied` is empty and `dropped_by_bounds` counts them all.
            receipt,
        );
    }
    (
        vec![ContextBlock {
            label: BLOCK_LABEL.to_string(),
            body,
            truncated: count_truncated || byte_truncated,
        }],
        receipt,
    )
}

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
async fn resolve_profiles(
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
