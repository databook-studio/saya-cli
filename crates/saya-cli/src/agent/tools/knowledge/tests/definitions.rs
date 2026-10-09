use super::*;

#[tokio::test]
async fn contract_tool_argument_validation_errors_are_typed() {
    let identity = profile_identity("primary");
    let tools =
        DatabaseTools::with_registry(registry_with_primary("primary", &identity), 100, true, None);

    let err = tools
        .execute(
            "contract_search",
            serde_json::json!({"terms": ["x"], "bogus": 1}),
        )
        .await
        .expect_err("unknown property must be rejected");
    assert!(matches!(err, ToolError::UnsupportedProperty), "got: {err}");

    let err = tools
        .execute(
            "contract_search",
            serde_json::json!({"terms": ["x"], "connection": 7}),
        )
        .await
        .expect_err("non-string connection must be rejected");
    assert!(matches!(err, ToolError::ConnectionNotString), "got: {err}");

    let terms: Vec<String> = (0..17).map(|i| format!("t{i}")).collect();
    let err = tools
        .execute("contract_search", serde_json::json!({"terms": terms}))
        .await
        .expect_err("17 terms must be rejected");
    assert!(
        matches!(err, ToolError::InvalidQueryArguments),
        "too many terms should be InvalidQueryArguments: {err}"
    );

    // terms not an array of strings is also a typed error.
    let err = tools
        .execute("contract_search", serde_json::json!({"terms": "orders"}))
        .await
        .expect_err("terms-as-string must be rejected");
    assert!(
        matches!(err, ToolError::InvalidQueryArguments),
        "got: {err}"
    );
}

#[test]
fn contract_tool_definitions_are_read_only_unapproved_and_not_writable() {
    let tools = DatabaseTools::definitions(true, true, false, false, true);
    let search = tools
        .iter()
        .find(|t| t.name == "contract_search")
        .expect("contract_search is registered when a store is present and data is allowed");
    let read = tools
        .iter()
        .find(|t| t.name == "contract_read")
        .expect("contract_read is registered");

    for tool in [search, read] {
        assert!(tool.read_only, "{} must be read_only", tool.name);
        assert!(
            !tool.effect.requires_approval,
            "{} must not require approval",
            tool.name
        );
        assert!(
            !tool.effect.external_side_effect,
            "{} must have no external side effect",
            tool.name
        );
        assert!(
            tool.effect.database_data,
            "{} is database-derived and subject to the sharing gate",
            tool.name
        );
    }
}

#[test]
fn contract_tools_are_hidden_without_a_store_or_when_the_gate_is_closed() {
    let with_store_and_gate = DatabaseTools::definitions(true, true, false, false, true);
    assert!(
        with_store_and_gate
            .iter()
            .any(|t| t.name == "contract_search")
    );
    assert!(
        with_store_and_gate
            .iter()
            .any(|t| t.name == "contract_read")
    );

    let no_store = DatabaseTools::definitions(true, false, false, false, true);
    assert!(!no_store.iter().any(|t| t.name == "contract_search"));
    assert!(!no_store.iter().any(|t| t.name == "contract_read"));

    let gate_closed = DatabaseTools::definitions(false, true, false, false, false);
    assert!(!gate_closed.iter().any(|t| t.name == "contract_search"));
    assert!(!gate_closed.iter().any(|t| t.name == "contract_read"));
}
