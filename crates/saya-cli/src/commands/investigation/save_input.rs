//! The save request's inputs, vetted (D3): resolves `--sql`/`--file`/stdin
//! through the shared reader, then gates the SQL — non-empty, the read-only
//! preparation gate (`prepare_for_dialect`, never execution), no
//! credential-shaped content — and assembles the v1 definition within its
//! bounds.

use super::connection::resolve_connection;
use super::{EXIT_INVESTIGATION_ERROR, EXIT_SAFETY};
use crate::config::runtime::RuntimeConfig;
use saya_connectors::prepare_for_dialect;
use saya_types::investigation::{
    INVESTIGATION_FORMAT, INVESTIGATION_FORMAT_VERSION, InvestigationDefinitionV1, InvestigationId,
};
use saya_types::{ProfileIdentity, redact};
use std::path::PathBuf;

/// One save request, as the adapter parsed it: the exact SQL from `--sql` or
/// `--file` (the shared stdin path fills `sql` when both are absent), the
/// display name/description, the connection to bind, if given, and the
/// declared `--param-spec` list.
pub(super) struct SaveRequest<'a> {
    pub name: &'a str,
    pub description: Option<&'a str>,
    pub sql: Option<String>,
    pub file: Option<PathBuf>,
    pub connection: Option<&'a str>,
    pub param_specs: &'a [String],
}

/// Validates the request into a v1 definition plus the resolved saving
/// profile's name and identity, refusing at the first gate as a
/// `(exit code, message)` pair.
pub(super) fn checked_definition(
    runtime: &RuntimeConfig,
    request: SaveRequest<'_>,
) -> Result<(InvestigationDefinitionV1, String, ProfileIdentity), (i32, String)> {
    let SaveRequest {
        name,
        description,
        sql,
        file,
        connection,
        param_specs,
    } = request;
    // The declarations parse first: a malformed spec is a usage error before
    // any SQL work (invariant 1).
    let parameters = super::params::parse_specs(param_specs)?;
    let (profile_name, dialect, identity) = resolve_connection(runtime, connection)?;
    if sql.is_some() && file.is_some() {
        return Err((
            EXIT_INVESTIGATION_ERROR,
            "pass --sql or --file, not both".to_string(),
        ));
    }
    // --sql/--file/stdin resolve exactly like `query` does.
    let sql = crate::commands::query_input::input(sql, file)
        .map_err(|error| (EXIT_INVESTIGATION_ERROR, error.to_string()))?;
    if sql.trim().is_empty() {
        return Err((
            EXIT_INVESTIGATION_ERROR,
            "SQL must be non-empty; pass --sql <SQL> or --file <PATH>".to_string(),
        ));
    }
    // The execution-time gate, without execution: preparation only, against a
    // one-row bound since no statement runs here.
    if let Err(error) = prepare_for_dialect(&sql, 1, dialect) {
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
    // The placeholder/declaration contract (invariant 1): every `:name` in
    // the SQL must be declared exactly once, and every declaration must
    // appear in the SQL — both ways, before the document exists.
    super::params::check_contract(&sql, dialect, &parameters)?;
    let now = now_unix_ms();
    let name = name.trim();
    // The informational objects field, canonically rendered from the SQL
    // (A2 decision 2): each part bare or double-quoted, parts joined with
    // "." — a dotted table name stays one object.
    let objects = super::objects::canonical_objects(&sql, dialect);
    let definition = InvestigationDefinitionV1 {
        format: INVESTIGATION_FORMAT.to_string(),
        version: INVESTIGATION_FORMAT_VERSION,
        id: InvestigationId::derive(name, &sql, now),
        revision: 1,
        name: name.to_string(),
        description: description.map(str::to_string),
        sql,
        parameters,
        dialect,
        connection: profile_name.clone(),
        objects,
        schema_fingerprint: None,
        created_unix_ms: now,
        updated_unix_ms: now,
    };
    definition
        .validate()
        .map_err(|error| (EXIT_INVESTIGATION_ERROR, error.to_string()))?;
    Ok((definition, profile_name, identity))
}

fn now_unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or_default()
}
