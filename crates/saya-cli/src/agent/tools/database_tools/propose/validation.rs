//! Argument shape-validation for `contract_propose`.
//!
//! Mirrors the read tools' validation: an unknown property is
//! `UnsupportedProperty`, a non-string `connection` is `ConnectionNotString`,
//! and every other shape failure (a missing or non-string `table`/`kind`/`value`,
//! a non-string `column`) maps to `InvalidQueryArguments`. `ToolError` lives in
//! `saya-agent` and has no more-specific variant for these; the value-level
//! checks (qualified-name arity, kind vocabulary, payload admission) run later,
//! in `crate::contracts::args`, also mapped to payload-free errors.

use saya_agent::ToolError;

/// The allowed properties on a `contract_propose` call, in declaration order.
pub(super) const ALLOWED_PROPERTIES: &[&str] = &["table", "kind", "value", "column", "connection"];

/// The shape-validated `contract_propose` arguments, as `&str` borrows into the
/// raw `arguments` so no untrusted value is copied before it is validated.
#[derive(Debug)]
pub(super) struct ProposeArgs<'a> {
    pub(super) table: &'a str,
    pub(super) kind: &'a str,
    pub(super) value: &'a str,
    pub(super) column: Option<&'a str>,
    pub(super) connection: Option<&'a str>,
}

/// Extracts and shape-validates the tool arguments. Shape failures map to
/// payload-free `ToolError` variants; the offending value is never echoed.
pub(super) fn parse_arguments(arguments: &serde_json::Value) -> Result<ProposeArgs<'_>, ToolError> {
    let object = arguments.as_object().ok_or(ToolError::ArgumentsNotObject)?;
    if object
        .keys()
        .any(|key| !ALLOWED_PROPERTIES.contains(&key.as_str()))
    {
        return Err(ToolError::UnsupportedProperty);
    }
    if object
        .get("connection")
        .is_some_and(|connection| !connection.is_string())
    {
        return Err(ToolError::ConnectionNotString);
    }
    let str_field = |key: &str| object.get(key).and_then(serde_json::Value::as_str);
    Ok(ProposeArgs {
        table: str_field("table").ok_or(ToolError::InvalidQueryArguments)?,
        kind: str_field("kind").ok_or(ToolError::InvalidQueryArguments)?,
        value: str_field("value").ok_or(ToolError::InvalidQueryArguments)?,
        column: str_field("column"),
        connection: str_field("connection"),
    })
}

/// Asserts (in tests) that the definition's `kind` enum and the allowed
/// properties match what this validator expects — a drift guard so the schema
/// and the validator cannot silently disagree.
#[cfg(test)]
pub(super) fn assert_definition_matches(definition: &saya_agent::ToolDefinition) {
    use super::definition::KIND_ENUM;
    let props = definition.parameters["properties"]
        .as_object()
        .expect("properties is an object");
    for allowed in ALLOWED_PROPERTIES {
        assert!(
            props.contains_key(*allowed),
            "definition is missing the `{allowed}` property"
        );
    }
    let kind_enum = definition.parameters["properties"]["kind"]["enum"]
        .as_array()
        .expect("kind has an enum");
    let advertised: Vec<&str> = kind_enum
        .iter()
        .filter_map(serde_json::Value::as_str)
        .collect();
    assert_eq!(
        advertised, *KIND_ENUM,
        "the kind enum the model sees must match the validator's vocabulary"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_arguments_rejects_unknown_property() {
        let err = parse_arguments(&serde_json::json!({"table": "a.b.c", "kind": "alias",
            "value": "v", "bogus": 1}))
        .unwrap_err();
        assert!(matches!(err, ToolError::UnsupportedProperty), "got: {err}");
    }

    #[test]
    fn parse_arguments_rejects_non_string_connection() {
        let err = parse_arguments(&serde_json::json!({"table": "a.b.c", "kind": "alias",
            "value": "v", "connection": 7}))
        .unwrap_err();
        assert!(matches!(err, ToolError::ConnectionNotString), "got: {err}");
    }

    #[test]
    fn parse_arguments_rejects_non_object() {
        let err = parse_arguments(&serde_json::json!(["a"])).unwrap_err();
        assert!(matches!(err, ToolError::ArgumentsNotObject), "got: {err}");
    }

    #[test]
    fn parse_arguments_requires_table_kind_value() {
        assert!(matches!(
            parse_arguments(&serde_json::json!({"kind": "alias", "value": "v"})).unwrap_err(),
            ToolError::InvalidQueryArguments
        ));
        assert!(matches!(
            parse_arguments(&serde_json::json!({"table": "a.b.c", "value": "v"})).unwrap_err(),
            ToolError::InvalidQueryArguments
        ));
        assert!(matches!(
            parse_arguments(&serde_json::json!({"table": "a.b.c", "kind": "alias"})).unwrap_err(),
            ToolError::InvalidQueryArguments
        ));
    }

    #[test]
    fn parse_arguments_accepts_optional_column_and_connection() {
        let raw = serde_json::json!({
            "table": "a.b.c", "kind": "column-description", "value": "v", "column": "amount"
        });
        let args = parse_arguments(&raw).unwrap();
        assert_eq!(args.column, Some("amount"));
        assert_eq!(args.connection, None);
    }
}
