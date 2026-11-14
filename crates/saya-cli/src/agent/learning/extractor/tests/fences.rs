use super::*;

#[test]
fn test_parse_handles_markdown_fenced_json() {
    let table = setup_test_table();
    let fenced = r#"```json
{
    "proposals": [
        {
            "object_id": "T0",
            "slot": "table.grain",
            "value": "one row per order",
            "origin": "user_explicit"
        }
    ]
}
```"#;

    let res = parse_extraction_response(fenced, &table).unwrap();
    assert_eq!(res.len(), 1);
    assert_eq!(res[0].object_id, TurnObjectId::new(0));
}

#[test]
fn test_parse_handles_malformed_json_gracefully() {
    let table = setup_test_table();
    let bad_json = r#"{"proposals": ["provider-json-secret-sentinel"}"#;

    let res = parse_extraction_response(bad_json, &table);
    assert!(res.is_err());
    match res.unwrap_err() {
        ExtractionError::JsonParse(diagnostic) => {
            assert!(diagnostic.contains("line"));
            assert!(diagnostic.contains("column"));
            assert!(diagnostic.len() <= MAX_JSON_DIAGNOSTIC_BYTES);
            assert!(!diagnostic.contains("provider-json-secret-sentinel"));
        }
        other => panic!("Expected JsonParse error, got {other:?}"),
    }
}

/// JSON mode returns bare JSON (no fence); the default
/// path returns ```` ```json ````-wrapped output. Not every provider honours
/// the JSON hint, so the stripper stays and both shapes must parse.
#[test]
fn test_parse_handles_bare_json() {
    let table = setup_test_table();
    let bare = r#"{"proposals": [
            {
                "object_id": "T0",
                "slot": "table.grain",
                "value": "one row per order",
                "origin": "user_explicit"
            }
        ]}"#;

    let res = parse_extraction_response(bare, &table).unwrap();
    assert_eq!(res.len(), 1);
    assert_eq!(res[0].object_id, TurnObjectId::new(0));
}

/// The stripper itself handles bare JSON, the ```json fence, and a bare ```
/// fence — the three shapes a provider that ignores the JSON hint can still
/// return. Both fences collapse to the same inner JSON; bare passes through.
#[test]
fn strip_markdown_fences_handles_bare_and_both_fence_flavors() {
    let bare = r#"  {"proposals": []}  "#;
    assert_eq!(strip_markdown_fences(bare), r#"{"proposals": []}"#);

    let json_fence = "```json\n{\"proposals\": []}\n```";
    assert_eq!(strip_markdown_fences(json_fence), r#"{"proposals": []}"#);

    let plain_fence = "```\n{\"proposals\": []}\n```";
    assert_eq!(strip_markdown_fences(plain_fence), r#"{"proposals": []}"#);
}
