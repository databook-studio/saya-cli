//! Proposal resolution and schema binding derivation / Chunk 2.
//!
//! Maps extracted turn-scoped proposals (`T0..Tn`) back to fully-resolved
//! `DatabaseObjectRef`s and derives structural `SchemaBinding`s from the
//! paired `(KnowledgeSlot, ClaimPayload)`.
//! Under-qualified object references are resolved against the active profile's
//! real `SchemaTree` (never fabricated with "default").

use crate::agent::learning::extractor_schema::{ExtractedProposal, ProposalOrigin};
use crate::agent::learning::gate::contains_assertion_intent;
use crate::agent::learning::profile_catalog::{ProfileCatalog, ProfileLookupError};
use crate::agent::learning::turn_table::{TurnObjectId, TurnObjectTable};
use crate::connection::ConnectionRegistry;
use saya_types::{
    ClaimOrigin, ClaimPayload, DatabaseObjectRef, KnowledgeSlot, KnowledgeState, SchemaBinding,
};
use thiserror::Error;

mod schema;

/// Maps a profile lookup failure onto the resolver's own error vocabulary.
fn lookup_error(profile: &str, error: ProfileLookupError) -> ResolutionError {
    match error {
        ProfileLookupError::Connection(detail) => {
            ResolutionError::ConnectionError(profile.to_string(), detail)
        }
        ProfileLookupError::MissingIdentity => {
            ResolutionError::MissingProfileIdentity(profile.to_string())
        }
        ProfileLookupError::InvalidIdentity(detail) => {
            ResolutionError::InvalidProfileIdentity(profile.to_string(), detail)
        }
    }
}

/// A proposal fully resolved to a database object reference, verified profile,
/// and derived schema binding.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub struct ResolvedProposal {
    pub object: DatabaseObjectRef,
    pub profile_name: String,
    pub slot: KnowledgeSlot,
    pub value: ClaimPayload,
    pub source: ClaimOrigin,
    pub state: KnowledgeState,
    pub schema_binding: SchemaBinding,
}

/// Errors occurring during proposal resolution.
#[derive(Debug, Error, PartialEq, Eq)]
#[allow(dead_code)]
pub enum ResolutionError {
    #[error("turn object id {0} not found in turn table")]
    ObjectIdNotFound(TurnObjectId),
    #[error("connection error resolving profile '{0}': {1}")]
    ConnectionError(String, String),
    #[error("connection entry for profile '{0}' has no profile id")]
    MissingProfileIdentity(String),
    #[error("invalid profile identity for '{0}': {1}")]
    InvalidProfileIdentity(String, String),
    #[error("invalid object reference '{0}': {1}")]
    InvalidObjectRef(String, String),
    #[error("object '{0}' cannot be resolved against profile schema")]
    UnresolvableObject(String),
    #[error("object reference '{0}' is ambiguous in profile schema")]
    AmbiguousObject(String),
    #[error("object '{0}' has no column '{1}'")]
    UnknownColumn(String, String),
    #[error("failed to derive schema binding for slot and payload")]
    InvalidSchemaBinding,
}

/// Resolves an extracted turn-scoped proposal into a [`ResolvedProposal`].
///
/// 1. Maps `extracted.object_id` to `(profile_name, qualified_name)` via `table`.
/// 2. Resolves the profile connection via `registry` and validates `ProfileIdentity`.
/// 3. Inspects the connection's real `SchemaTree` to resolve under-qualified references
///    without fabricating missing catalog or schema names.
/// 4. Derives `SchemaBinding` from `(slot, value)`; returns `Err` if invalid.
/// 5. Refuses a proposal whose schema binding names a column the resolved object
///    does not have (`ResolutionError::UnknownColumn`).
/// 6. Maps origin:
///    - `ProposalOrigin::UserExplicit` -> `(KnowledgeState::Active, ClaimOrigin::UserExplicit)`
///      when the turn prompt shows an assertion (`contains_assertion_intent`); without
///      that corroboration the proposal resolves like any inference
///      (`Pending`, `ClaimOrigin::AssistantInferred`)
///    - `ProposalOrigin::AssistantInferred` -> `(KnowledgeState::Pending, ClaimOrigin::AssistantInferred)`
#[allow(dead_code)]
pub async fn resolve_proposal(
    extracted: ExtractedProposal,
    prompt: &str,
    table: &TurnObjectTable,
    registry: &ConnectionRegistry,
) -> Result<ResolvedProposal, ResolutionError> {
    let mut catalog = ProfileCatalog::new();
    resolve_with_catalog(extracted, prompt, table, registry, &mut catalog).await
}

/// Resolves every proposal from one turn, reading each profile's schema once.
///
/// A proposal that cannot be resolved is dropped rather than fabricated, so the
/// returned vector may be shorter than the input.
pub(crate) async fn resolve_proposals(
    extracted: Vec<ExtractedProposal>,
    prompt: &str,
    table: &TurnObjectTable,
    registry: &ConnectionRegistry,
) -> Vec<ResolvedProposal> {
    let mut catalog = ProfileCatalog::new();
    let mut resolved = Vec::with_capacity(extracted.len());
    for proposal in extracted {
        if let Ok(one) = resolve_with_catalog(proposal, prompt, table, registry, &mut catalog).await
        {
            resolved.push(one);
        }
    }
    resolved
}

async fn resolve_with_catalog(
    extracted: ExtractedProposal,
    prompt: &str,
    table: &TurnObjectTable,
    registry: &ConnectionRegistry,
    catalog: &mut ProfileCatalog,
) -> Result<ResolvedProposal, ResolutionError> {
    let entry = table
        .get_by_id(&extracted.object_id)
        .ok_or(ResolutionError::ObjectIdNotFound(extracted.object_id))?;

    let facts = catalog
        .facts(&entry.profile, registry)
        .await
        .map_err(|e| lookup_error(&entry.profile, e))?;

    let object =
        schema::resolve_database_object(&facts.identity, &facts.schema, &entry.qualified_name)?;

    let schema_binding = SchemaBinding::derive(&extracted.slot, &extracted.value)
        .ok_or(ResolutionError::InvalidSchemaBinding)?;

    schema::validate_binding_columns(&facts.schema, &object, &schema_binding)?;

    let (state, source) = match extracted.origin {
        ProposalOrigin::UserExplicit => {
            if contains_assertion_intent(prompt) {
                (KnowledgeState::Active, ClaimOrigin::UserExplicit)
            } else {
                // The origin comes from the model's output, so an assertion the
                // turn does not show is uncorroborated: the proposal resolves
                // like any inference rather than confirming itself, and the
                // stored claim must not carry a user provenance that never
                // happened.
                (KnowledgeState::Pending, ClaimOrigin::AssistantInferred)
            }
        }
        ProposalOrigin::AssistantInferred => {
            (KnowledgeState::Pending, ClaimOrigin::AssistantInferred)
        }
    };

    Ok(ResolvedProposal {
        object,
        profile_name: entry.profile.clone(),
        slot: extracted.slot,
        value: extracted.value,
        source,
        state,
        schema_binding,
    })
}

#[cfg(test)]
#[path = "tests/mod.rs"]
mod tests;
