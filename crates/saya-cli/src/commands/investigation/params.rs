//! The `--param-spec` declarations and the placeholder/declaration
//! contract for the investigation surfaces (B1f). The declaration grammar is
//! `name:type[:required]`; the contract is checked with the safety layer's
//! own placeholder walker (`sql_placeholders`), so a saved document's SQL
//! and its declarations can never disagree. No error text echoes a value —
//! only names, which already appear in the SQL.

use super::EXIT_INVESTIGATION_ERROR;
use saya_connectors::sql_placeholders;
use saya_types::{ParamType, ParameterSpec, SqlDialect};

/// Parses the `--param-spec` declarations: each is `name:type[:required]`
/// with type one of string|integer|boolean|decimal|date|timestamp and the
/// third part the literal `required` (the default is optional). The whole
/// list is checked for the per-spec bounds, duplicates, and the 32 cap.
pub(super) fn parse_specs(raw: &[String]) -> Result<Vec<ParameterSpec>, (i32, String)> {
    let specs = raw
        .iter()
        .map(|text| parse_spec(text))
        .collect::<Result<Vec<_>, _>>()?;
    ParameterSpec::validate_list(&specs)
        .map_err(|error| (EXIT_INVESTIGATION_ERROR, error.to_string()))?;
    Ok(specs)
}

fn parse_spec(raw: &str) -> Result<ParameterSpec, (i32, String)> {
    const USAGE: &str = "use --param-spec name:type[:required] with type string|integer|boolean|decimal|date|timestamp";
    let mut parts = raw.split(':');
    let name = parts.next().unwrap_or_default();
    let Some(type_tag) = parts.next().filter(|tag| !tag.is_empty()) else {
        return Err((EXIT_INVESTIGATION_ERROR, format!("{USAGE}: {raw}")));
    };
    let rest: Vec<&str> = parts.collect();
    let (required, extra) = match rest.as_slice() {
        [] => (false, None),
        ["required"] => (true, None),
        [other] => (false, Some(*other)),
        _ => (false, Some("more than one")),
    };
    if let Some(extra) = extra {
        return Err((
            EXIT_INVESTIGATION_ERROR,
            format!(
                "{USAGE}: {raw} (the optional third part is exactly \"required\", got {extra:?})"
            ),
        ));
    }
    let Some(param_type) = param_type_of(type_tag) else {
        return Err((
            EXIT_INVESTIGATION_ERROR,
            format!("{USAGE}: unknown type {type_tag:?}"),
        ));
    };
    ParameterSpec::new(name, param_type, required, None)
        .map_err(|error| (EXIT_INVESTIGATION_ERROR, format!("{USAGE}: {error}")))
}

fn param_type_of(tag: &str) -> Option<ParamType> {
    [
        ParamType::String,
        ParamType::Integer,
        ParamType::Boolean,
        ParamType::Decimal,
        ParamType::Date,
        ParamType::Timestamp,
    ]
    .into_iter()
    .find(|param_type| param_type.as_str() == tag)
}

/// The placeholder/declaration contract for a saved document: the SQL's
/// distinct `:name` placeholders must equal the declared names, both ways,
/// and any other placeholder form is refused. One refusal names every
/// missing and every extra name — never a value.
pub(super) fn check_contract(
    sql: &str,
    dialect: SqlDialect,
    specs: &[ParameterSpec],
) -> Result<(), (i32, String)> {
    let placeholders = sql_placeholders(sql, dialect)
        .map_err(|error| (EXIT_INVESTIGATION_ERROR, error.to_string()))?;
    let declared: Vec<&str> = specs.iter().map(|spec| spec.name.as_str()).collect();
    let missing: Vec<&str> = placeholders
        .iter()
        .map(String::as_str)
        .filter(|name| !declared.contains(name))
        .collect();
    let extra: Vec<&str> = declared
        .iter()
        .copied()
        .filter(|name| !placeholders.iter().any(|in_sql| in_sql == *name))
        .collect();
    if missing.is_empty() && extra.is_empty() {
        return Ok(());
    }
    let mut message = String::new();
    if !missing.is_empty() {
        message.push_str(&format!(
            "the SQL uses placeholder(s) {} that no --param-spec declares",
            missing.join(", ")
        ));
    }
    if !extra.is_empty() {
        if !message.is_empty() {
            message.push_str("; ");
        }
        message.push_str(&format!(
            "--param-spec declares parameter(s) the SQL does not use: {}",
            extra.join(", ")
        ));
    }
    Err((EXIT_INVESTIGATION_ERROR, message))
}

/// The engine name the capability refusal uses, matching the safety layer's
/// wording for the same refusal (`safety/params.rs`): ClickHouse by name,
/// the other dialects by their debug name.
pub(super) fn parameters_engine_name(dialect: SqlDialect) -> String {
    match dialect {
        SqlDialect::ClickHouse => "ClickHouse".to_owned(),
        other => format!("the {other:?} dialect"),
    }
}

#[cfg(test)]
#[path = "params_tests.rs"]
mod tests;
