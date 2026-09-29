//! The run-time `--param` bindings (B1f): values parse against the declared
//! types and come back in declaration order for `QueryRequest::with_params`.
//! Values are parsed, bound, and dropped — nothing here keeps one beyond the
//! request; the only trace is the evidence digest, which hashes the
//! canonical `name=value` lines and never carries a value. No error text
//! echoes a value either.

use super::EXIT_INVESTIGATION_ERROR;
use saya_types::{BoundParam, ParamValue, ParameterSpec};
use sha2::{Digest, Sha256};

/// Binds the `--param name=value` values against the declared specs: every
/// name must be declared (refused otherwise, listing the declarations), each
/// value parses as its declared type, a missing required parameter refuses
/// with the required names and types, and an omitted optional binds as a
/// typed null. The result is one binding per declared spec, in declaration
/// order — the canonical order the evidence digest hashes.
pub(super) fn bind_values(
    specs: &[ParameterSpec],
    raw: &[String],
) -> Result<Vec<BoundParam>, (i32, String)> {
    let mut supplied: Vec<(&str, &str)> = Vec::new();
    for text in raw {
        let Some((name, value)) = text.split_once('=') else {
            return Err((
                EXIT_INVESTIGATION_ERROR,
                "--param needs name=value".to_string(),
            ));
        };
        if supplied.iter().any(|(bound, _)| *bound == name) {
            return Err((
                EXIT_INVESTIGATION_ERROR,
                format!("the parameter {name:?} was bound more than once"),
            ));
        }
        if !specs.iter().any(|spec| spec.name == name) {
            let declared = if specs.is_empty() {
                "this investigation declares no parameters".to_string()
            } else {
                format!(
                    "declared parameters: {}",
                    specs
                        .iter()
                        .map(|spec| format!("{} ({})", spec.name, spec.param_type))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            };
            return Err((
                EXIT_INVESTIGATION_ERROR,
                format!("no parameter named {name:?} is declared; {declared}"),
            ));
        }
        supplied.push((name, value));
    }
    let missing: Vec<String> = specs
        .iter()
        .filter(|spec| spec.required && !supplied.iter().any(|(name, _)| *name == spec.name))
        .map(|spec| format!("{} ({})", spec.name, spec.param_type))
        .collect();
    if !missing.is_empty() {
        return Err((
            EXIT_INVESTIGATION_ERROR,
            format!(
                "missing required parameter(s): {} — pass each as --param <name>=<value>",
                missing.join(", ")
            ),
        ));
    }
    specs
        .iter()
        .map(|spec| {
            let value = match supplied.iter().find(|(name, _)| *name == spec.name) {
                Some((_, value)) => ParamValue::parse(spec.param_type, value).map_err(|error| {
                    (
                        EXIT_INVESTIGATION_ERROR,
                        format!("parameter {:?}: {error}", spec.name),
                    )
                })?,
                None => ParamValue::Null(spec.param_type),
            };
            Ok(BoundParam {
                name: spec.name.clone(),
                value,
            })
        })
        .collect()
}

/// The canonical text one bound value contributes to the digest: its own
/// validated text, with a null printed as `null`.
fn canonical_value_text(value: &ParamValue) -> String {
    match value {
        ParamValue::Null(_) => "null".to_owned(),
        ParamValue::String(text)
        | ParamValue::Decimal(text)
        | ParamValue::Date(text)
        | ParamValue::Timestamp(text) => text.clone(),
        ParamValue::Integer(int) => int.to_string(),
        ParamValue::Boolean(flag) => flag.to_string(),
    }
}

/// The evidence fields for bound parameters: the names and the sha256 over
/// the canonical `name=value` lines in declaration order — never a value.
/// Empty bindings carry no fields.
pub(super) fn evidence_fields(binds: &[BoundParam]) -> (Vec<String>, Option<String>) {
    if binds.is_empty() {
        return (Vec::new(), None);
    }
    let canonical = binds
        .iter()
        .map(|param| format!("{}={}", param.name, canonical_value_text(&param.value)))
        .collect::<Vec<_>>()
        .join("\n");
    let digest = {
        let mut hash = Sha256::new();
        hash.update(canonical.as_bytes());
        hash.finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    };
    (
        binds.iter().map(|param| param.name.clone()).collect(),
        Some(digest),
    )
}

#[cfg(test)]
#[path = "bindings_tests.rs"]
mod tests;
