//! Tests for the `--param-spec` declarations and the placeholder/declaration
//! contract: the grammar, the bounds, and the both-ways placeholder match.

use super::{check_contract, parse_specs};
use saya_types::{ParamType, ParameterSpec, SqlDialect};

fn spec_named(name: &str, param_type: ParamType, required: bool) -> ParameterSpec {
    ParameterSpec::new(name, param_type, required, None).unwrap()
}

// --- parse_specs ------------------------------------------------------------

#[test]
fn spec_grammar_parses_type_and_the_required_third_part() {
    let specs = parse_specs(&["region:string:required".to_owned()]).unwrap();
    assert_eq!(specs, vec![spec_named("region", ParamType::String, true)]);

    let specs = parse_specs(&["day:date".to_owned()]).unwrap();
    assert_eq!(specs, vec![spec_named("day", ParamType::Date, false)]);

    for (tag, param_type) in [
        ("string", ParamType::String),
        ("integer", ParamType::Integer),
        ("boolean", ParamType::Boolean),
        ("decimal", ParamType::Decimal),
        ("date", ParamType::Date),
        ("timestamp", ParamType::Timestamp),
    ] {
        let specs = parse_specs(&[format!("p:{tag}")]).unwrap();
        assert_eq!(specs[0].param_type, param_type, "{tag}");
    }
}

#[test]
fn spec_grammar_refuses_malformed_declarations() {
    for raw in [
        "region",
        ":string",
        "region:",
        "region:str",
        "region:string:always",
        "region:string:required:extra",
        "Region:string",
        "region::required",
    ] {
        let (code, message) = parse_specs(&[raw.to_owned()]).unwrap_err();
        assert_eq!(code, 2, "{raw}");
        assert!(
            message.contains("param-spec"),
            "the refusal names the flag grammar: {message}"
        );
    }
}

#[test]
fn spec_list_refuses_duplicates_and_caps_the_count() {
    let (code, message) = parse_specs(&["a:string".to_owned(), "a:string".to_owned()]).unwrap_err();
    assert_eq!(code, 2, "{message}");
    assert!(
        message.contains("a"),
        "the refusal names the duplicate: {message}"
    );

    let many: Vec<String> = (0..33).map(|i| format!("p{i:02}:string")).collect();
    let (code, message) = parse_specs(&many).unwrap_err();
    assert_eq!(code, 2, "{message}");
    assert!(message.contains("32"), "{message}");
}

// --- check_contract ---------------------------------------------------------

#[test]
fn contract_accepts_matching_declarations() {
    let specs = parse_specs(&["region:string".to_owned(), "floor:integer".to_owned()]).unwrap();
    check_contract(
        "SELECT 1 FROM t WHERE r = :region AND f > :floor AND f < :floor",
        SqlDialect::Sqlite,
        &specs,
    )
    .unwrap();
    check_contract("SELECT 1 FROM t", SqlDialect::Sqlite, &[]).unwrap();
}

#[test]
fn contract_names_missing_and_extra_declarations() {
    let specs = parse_specs(&["region:string".to_owned()]).unwrap();
    let (code, message) = check_contract(
        "SELECT 1 FROM t WHERE c = :city",
        SqlDialect::Sqlite,
        &specs,
    )
    .unwrap_err();
    assert_eq!(code, 2);
    assert!(
        message.contains("city") && message.contains("param-spec"),
        "the refusal names the undeclared placeholder: {message}"
    );

    let (code, message) =
        check_contract("SELECT 1 FROM t", SqlDialect::Sqlite, &specs).unwrap_err();
    assert_eq!(code, 2);
    assert!(
        message.contains("region"),
        "the refusal names the unused declaration: {message}"
    );

    let (_, message) = check_contract(
        "SELECT 1 FROM t WHERE c = :city",
        SqlDialect::Sqlite,
        &specs,
    )
    .unwrap_err();
    assert!(
        message.contains("city") && message.contains("region"),
        "both directions are named together: {message}"
    );
}

#[test]
fn contract_refuses_non_name_placeholder_forms() {
    let (code, message) =
        check_contract("SELECT 1 FROM t WHERE c = $1", SqlDialect::Postgres, &[]).unwrap_err();
    assert_eq!(code, 2);
    assert!(
        message.contains(":name"),
        "the refusal names the placeholder grammar: {message}"
    );
}

#[test]
fn contract_ignores_placeholders_inside_strings_and_comments() {
    // A `:name` inside a string literal or a comment is text, not a
    // placeholder: an unparameterized statement stays valid without specs.
    check_contract(
        "SELECT ':not_a_param' FROM t -- :also_not",
        SqlDialect::Sqlite,
        &[],
    )
    .unwrap();
}
