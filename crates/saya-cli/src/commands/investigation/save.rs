//! `saya investigation save` (D3): stores an exact SQL statement as a
//! portable investigation document and records the saving profile as the
//! local review binding. The SQL is vetted in `save_input.rs` — including by
//! the read-only gate execution uses — but is never executed, and nothing
//! here connects to a database.

use super::save_input::{SaveRequest, checked_definition};
use super::{EXIT_INVESTIGATION_ERROR, store_error_parts, store_failure};
use crate::commands::output::{failure_message, result};
use crate::config::runtime::RuntimeConfig;
use crate::render::RenderFormat;
use saya_store::{InvestigationRepository, LocalBinding, StoreError};
use saya_types::ProfileIdentity;
use saya_types::investigation::InvestigationDefinitionV1;

/// What save always says when it succeeds: the document keeps literals
/// verbatim, so sharing needs review first.
const SAVED_NOTE: &str =
    "Saved exactly as shown. Review the SQL before sharing: literals are stored verbatim.";

/// The save flow: vet the request, create the document, record the binding,
/// and print the id, the exact definition, and the review-before-sharing note.
pub(super) fn save(
    repo: &InvestigationRepository,
    runtime: &RuntimeConfig,
    format: RenderFormat,
    request: SaveRequest<'_>,
) -> Result<i32, Box<dyn std::error::Error>> {
    let (definition, profile_name, identity) = match checked_definition(runtime, request) {
        Ok(parts) => parts,
        Err((code, message)) => return failure_message(code, message, format),
    };
    match repo.create(&definition) {
        Ok(()) => {}
        // The id is derived (name + sql + save time), so a conflict is a
        // millisecond collision, not a guessable namespace.
        Err(StoreError::Conflict) => {
            return failure_message(
                EXIT_INVESTIGATION_ERROR,
                format!(
                    "an investigation with id {} already exists; list or show it instead",
                    definition.id.as_str()
                ),
                format,
            );
        }
        Err(error) => return store_failure(error, definition.id.as_str(), format),
    }
    if let Err(error) = repo.put_binding(&binding_of(&definition, profile_name, identity)) {
        let (code, store_message) = store_error_parts(&error, definition.id.as_str());
        let id = definition.id.as_str();
        return failure_message(
            code,
            format!("saved {id} but the local binding could not be written: {store_message}"),
            format,
        );
    }
    let json = definition.to_json_pretty()?;
    result(
        format!("{}\n{}\n{SAVED_NOTE}", definition.id.as_str(), json),
        format,
    )
}

/// The per-machine state beside the document (D2): saving records that the
/// saving profile reviewed exactly this revision, with this fingerprint, at
/// the document's creation time. Never exported.
fn binding_of(
    definition: &InvestigationDefinitionV1,
    profile_name: String,
    identity: ProfileIdentity,
) -> LocalBinding {
    LocalBinding {
        version: LocalBinding::VERSION,
        id: definition.id.clone(),
        profile: profile_name,
        profile_identity: identity.as_str().to_string(),
        reviewed_revision: definition.revision,
        reviewed_schema_fingerprint: definition.schema_fingerprint.clone(),
        reviewed_unix_ms: definition.created_unix_ms,
    }
}
