use super::*;

// ---------------------------------------------------------------------------
// Provenance: only a tool observation certifies a fact
// ---------------------------------------------------------------------------

#[test]
fn a_model_asserted_provenance_is_refused_not_ignored() {
    let json = format!(
        r#"{{"version":1,"source":{{"through_turn":1,"source_sha256":"{VALID_SHA256}"}},"files":[{{"path":"src/main.rs","observed_unix_ms":1700000000000,"provenance":{{"kind":"model_asserted"}}}}],"checks":[]}}"#
    );
    assert_eq!(
        SessionContinuationV1::from_json_bytes(json.as_bytes()),
        Err(ContinuationError::Malformed)
    );
    let tool = json.replace(
        r#""kind":"model_asserted""#,
        r#""kind":"tool_observed","tool":"saya""#,
    );
    SessionContinuationV1::from_json_bytes(tool.as_bytes())
        .expect("the tool-observed twin is the grant-free shape and parses");
}

#[test]
fn tool_observed_provenance_is_tagged_kind() {
    let json = serde_json::to_string(&file("p").provenance).unwrap();
    assert_eq!(json, r#"{"kind":"tool_observed","tool":"saya_test_tool"}"#);
}

#[test]
fn an_unname_shaped_tool_is_refused() {
    let mut record = minimal();
    record.files = vec![file("src/main.rs")];
    record.files[0].provenance = FactProvenance::ToolObserved {
        tool: String::new(),
    };
    assert_eq!(record.validate(), Err(ContinuationError::InvalidToolName));
    record.files[0].provenance = FactProvenance::ToolObserved {
        tool: "bad tool".to_string(),
    };
    assert_eq!(record.validate(), Err(ContinuationError::InvalidToolName));
    record.files[0].provenance = FactProvenance::ToolObserved {
        tool: "tool\u{0}".to_string(),
    };
    assert_eq!(record.validate(), Err(ContinuationError::InvalidToolName));
}
