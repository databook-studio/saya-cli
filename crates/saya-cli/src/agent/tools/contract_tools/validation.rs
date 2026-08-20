//! Argument validation for the read-only contract tools.
//!
//! Mirrors the database tools' `validate_arguments`: an unknown property is
//! `UnsupportedProperty`, a non-string `connection` is `ConnectionNotString`.
//! Every other shape failure (a non-array `terms`, a non-string element, more
//! than `MAX_TERMS` entries, a non-string `table`) maps to `InvalidQueryArguments`
//! — `ToolError` has no more-specific variant for these and lives in
//! `saya-agent`, which this slice does not touch; see the SPEC REVIEW for 2b-3a.

use saya_agent::ToolError;

use super::definitions::MAX_TERMS;

pub(super) fn validate_arguments(
    name: &str,
    arguments: &serde_json::Value,
) -> Result<(), ToolError> {
    let object = arguments.as_object().ok_or(ToolError::ArgumentsNotObject)?;
    let allowed = match name {
        "contract_search" => &["connection", "terms"][..],
        "contract_read" => &["connection", "table"][..],
        _ => return Err(ToolError::UnsupportedTool),
    };
    if object.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(ToolError::UnsupportedProperty);
    }
    if object
        .get("connection")
        .is_some_and(|connection| !connection.is_string())
    {
        return Err(ToolError::ConnectionNotString);
    }
    match name {
        "contract_search" => validate_terms(object.get("terms")),
        "contract_read" => validate_table(object.get("table")),
        _ => Err(ToolError::UnsupportedTool),
    }
}

fn validate_terms(terms: Option<&serde_json::Value>) -> Result<(), ToolError> {
    // Missing `terms` is treated as an empty search (returns nothing) rather
    // than an error — the JSON schema marks it `required` for the model, and an
    // empty search is a safe no-op, not a shape violation.
    let Some(terms) = terms else {
        return Ok(());
    };
    let array = terms.as_array().ok_or(ToolError::InvalidQueryArguments)?;
    if array.len() > MAX_TERMS {
        return Err(ToolError::InvalidQueryArguments);
    }
    if !array.iter().all(serde_json::Value::is_string) {
        return Err(ToolError::InvalidQueryArguments);
    }
    Ok(())
}

fn validate_table(table: Option<&serde_json::Value>) -> Result<(), ToolError> {
    // A missing or non-string `table` is a shape violation; a string that is
    // not three dot-separated parts is parsed (and rejected) at execution time,
    // also as `InvalidQueryArguments`.
    match table {
        Some(value) if value.is_string() => Ok(()),
        _ => Err(ToolError::InvalidQueryArguments),
    }
}
