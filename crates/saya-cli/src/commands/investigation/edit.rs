//! `saya investigation edit` (A2.5 reopened): replaces the edited fields of
//! a saved investigation as a new revision through
//! `InvestigationDefinitionV1::new_revision` and the repository's optimistic
//! update. The id, dialect, and connection alias are immutable; the
//! replacement SQL passes the same save-time gates as `save_input.rs` (the
//! read-only preparation gate and the credential-shape refusal) and the
//! canonical objects are recomputed from it. The local review binding is
//! deliberately left on the old revision — the next run refuses as stale
//! until `--revalidate`. Nothing here connects to a database and no SQL is
//! executed.

use super::objects;
use super::{EXIT_INVESTIGATION_ERROR, EXIT_SAFETY, parse_investigation_id, store_failure};
use crate::commands::output::{failure_message, result};
use crate::render::RenderFormat;
use saya_connectors::prepare_for_dialect;
use saya_store::InvestigationRepository;
use saya_types::investigation::InvestigationDefinitionV1;
use saya_types::redact;
use std::path::PathBuf;

/// One edit request, as the adapter parsed it: the id plus the fields to
/// replace; every absent field stays exactly as stored.
pub(super) struct EditRequest<'a> {
    pub id: &'a str,
    pub name: Option<&'a str>,
    pub description: Option<&'a str>,
    pub sql: Option<String>,
    pub file: Option<PathBuf>,
}

/// The edit flow: refuse a no-op up front, read the current document, apply
/// the changed fields under the save-time gates, and publish the next
/// revision with the store's optimistic check — a document that moved
/// underneath is a conflict, never an overwrite.
pub(super) fn edit(
    repo: &InvestigationRepository,
    format: RenderFormat,
    request: EditRequest<'_>,
) -> Result<i32, Box<dyn std::error::Error>> {
    if request.name.is_none()
        && request.description.is_none()
        && request.sql.is_none()
        && request.file.is_none()
    {
        return failure_message(
            EXIT_INVESTIGATION_ERROR,
            "edit needs a change: pass --name, --description, --sql, or --file".to_string(),
            format,
        );
    }
    let id = match parse_investigation_id(request.id) {
        Ok(id) => id,
        Err((code, message)) => return failure_message(code, message, format),
    };
    let current = match repo.get(&id) {
        Ok(current) => current,
        Err(error) => return store_failure(error, id.as_str(), format),
    };
    let (sql, name, description) = match changed_fields(&current, request) {
        Ok(fields) => fields,
        Err((code, message)) => return failure_message(code, message, format),
    };
    let mut next = current.new_revision(sql, name, description, now_unix_ms());
    next.objects = objects::canonical_objects(&next.sql, next.dialect);
    if let Err(error) = next.validate() {
        return failure_message(EXIT_INVESTIGATION_ERROR, error.to_string(), format);
    }
    if let Err(error) = repo.update(&next, current.revision) {
        return store_failure(error, id.as_str(), format);
    }
    let json = next.to_json_pretty()?;
    result(
        format!(
            "{json}\nEdited to revision {}. The next run needs --revalidate.",
            next.revision
        ),
        format,
    )
}

/// Resolves the edited fields: the replacement SQL — from `--sql`/`--file`,
/// or the stored SQL when neither is given — then the name (trimmed like
/// save) and the description. The stored SQL is re-gated too, so an edit of
/// any field still proves the document it publishes passes the execution
/// gate.
fn changed_fields(
    current: &InvestigationDefinitionV1,
    request: EditRequest<'_>,
) -> Result<(String, String, Option<String>), (i32, String)> {
    let sql = if request.sql.is_some() || request.file.is_some() {
        if request.sql.is_some() && request.file.is_some() {
            return Err((
                EXIT_INVESTIGATION_ERROR,
                "pass --sql or --file, not both".to_string(),
            ));
        }
        crate::commands::query_input::input(request.sql, request.file)
            .map_err(|error| (EXIT_INVESTIGATION_ERROR, error.to_string()))?
    } else {
        current.sql.clone()
    };
    if sql.trim().is_empty() {
        return Err((
            EXIT_INVESTIGATION_ERROR,
            "SQL must be non-empty; pass --sql <SQL> or --file <PATH>".to_string(),
        ));
    }
    // The execution-time gate, without execution: preparation only, against
    // a one-row bound since no statement runs here.
    if let Err(error) = prepare_for_dialect(&sql, 1, current.dialect) {
        return Err((EXIT_SAFETY, error.to_string()));
    }
    // Credential-shaped content is refused, not redacted: the document keeps
    // SQL exactly, so redacting would silently change what a replay runs.
    if redact(&sql) != sql {
        return Err((
            EXIT_INVESTIGATION_ERROR,
            "SQL contains credential-shaped text; saved SQL is kept exactly, so remove it"
                .to_string(),
        ));
    }
    let name = request
        .name
        .map_or_else(|| current.name.clone(), |name| name.trim().to_string());
    let description = request.description.map_or_else(
        || current.description.clone(),
        |description| Some(description.to_string()),
    );
    Ok((sql, name, description))
}

fn now_unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
#[path = "edit_tests.rs"]
mod tests;
