use super::*;

/// The parameter fields default to absent for a parameter-free run, and a
/// bound run carries names and a digest — never a value (B1f).
#[test]
fn parameter_fields_default_absent_and_carry_names_with_a_digest_only() {
    let bare = evidence();
    assert!(bare.param_names.is_empty());
    assert_eq!(bare.params_sha256, None);
    let json = serde_json::to_string(&bare).unwrap();
    assert!(
        !json.contains("param_names") && !json.contains("params_sha256"),
        "empty parameter fields are omitted from serialization: {json}"
    );

    let bound = evidence().with_param_bindings(
        vec!["label".to_owned(), "since".to_owned()],
        Some("a".repeat(64)),
    );
    assert_eq!(
        bound.param_names,
        vec!["label".to_owned(), "since".to_owned()]
    );
    assert_eq!(bound.params_sha256.as_deref(), Some(&"a".repeat(64)[..]));
    let json = serde_json::to_string(&bound).unwrap();
    assert!(
        json.contains("\"param_names\":[\"label\",\"since\"]"),
        "{json}"
    );
    assert!(json.contains("\"params_sha256\""), "{json}");
    assert!(
        !json.contains("value"),
        "the serialized evidence carries names and a digest, never a value: {json}"
    );

    // The digest round-trips: an evidence record deserializes with both
    // fields intact.
    let back: ExecutionEvidence =
        serde_json::from_str(&serde_json::to_string(&bound).unwrap()).unwrap();
    assert_eq!(back, bound);
}

/// The evidence line records the parameters' names — never a value.
#[test]
fn human_line_records_parameter_names_only() {
    let line = evidence()
        .with_param_bindings(vec!["label".to_owned()], Some("b".repeat(64)))
        .human_line();
    assert!(line.contains("params: label"), "{line}");
    assert!(!line.contains("params_sha256"), "{line}");
    let bare = evidence().human_line();
    assert!(!bare.contains("params:"), "{bare}");
}
