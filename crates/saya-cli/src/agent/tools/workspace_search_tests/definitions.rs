/// All three are advertised regardless of the data-sharing gate (they touch
/// no database data) and are read-shaped: read-only approval permits them,
/// they declare `LocalStateEffect::Read`, and each states its own completion —
/// the generic read-only wording says "database", which these are not.
#[test]
fn workspace_search_definitions_are_read_shaped_and_always_advertised() {
    let tools = DatabaseTools::definitions(false, false, false, false, false);
    let completions = [
        ("workspace_list", "workspace directory listed"),
        ("glob", "workspace paths matched"),
        ("grep", "workspace text searched"),
    ];
    for (name, completion) in completions {
        let tool = tools
            .iter()
            .find(|tool| tool.name == name)
            .expect("each workspace search tool is advertised even with the data gate closed");
        assert!(tool.read_only);
        assert!(!tool.effect.database_data);
        assert!(!tool.effect.external_side_effect);
        assert!(!tool.effect.requires_approval);
        assert_eq!(tool.effect.local_state, LocalStateEffect::Read);
        assert_eq!(
            tool.parameters["additionalProperties"],
            serde_json::Value::Bool(false)
        );
        assert_eq!(tool.completion.as_deref(), Some(completion));
    }
    let required = |name: &str| -> Vec<String> {
        tools
            .iter()
            .find(|tool| tool.name == name)
            .expect("definition exists")
            .parameters["required"]
            .as_array()
            .expect("required list")
            .iter()
            .map(|value| value.as_str().expect("name").to_string())
            .collect()
    };
    assert_eq!(required("workspace_list"), Vec::<String>::new());
    assert_eq!(required("glob"), vec!["pattern".to_string()]);
    assert_eq!(required("grep"), vec!["pattern".to_string()]);
}
