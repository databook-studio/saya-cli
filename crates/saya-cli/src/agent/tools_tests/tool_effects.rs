/// Spec 3a §2 / 3c: every existing tool declares the expected `local_state`.
/// This is the test that fails when someone adds a tool without saying what
/// local state it touches. With query data and a state store but candidate
/// writes off, the six pre-3c tools are present and none writes local state.
#[test]
fn every_tool_declares_its_local_state_effect() {
    use saya_agent::LocalStateEffect;

    let tools = DatabaseTools::definitions(true, true, false, false, true);
    let expected = [
        ("schema_discovery", LocalStateEffect::None),
        ("bounded_sql_query", LocalStateEffect::None),
        ("bounded_sql_query_all", LocalStateEffect::None),
        ("result_shape", LocalStateEffect::None),
        ("column_health", LocalStateEffect::None),
        ("join_check", LocalStateEffect::None),
        ("render_chart", LocalStateEffect::None),
        ("contract_search", LocalStateEffect::Read),
        ("contract_read", LocalStateEffect::Read),
    ];
    for (name, want) in expected {
        let tool = tools
            .iter()
            .find(|tool| tool.name == name)
            .unwrap_or_else(|| panic!("{name} must be registered"));
        assert_eq!(
            tool.effect.local_state, want,
            "{name} must declare local_state == {want:?}"
        );
    }
    // With writes off, contract_propose is hidden, so none writes local state.
    assert!(
        !tools
            .iter()
            .any(|tool| tool.effect.local_state == LocalStateEffect::WriteCandidate),
        "no tool may declare WriteCandidate when writes are not permitted"
    );

    // The workspace write tools declare `WriteWorkspace` when permitted.
    // `workspace_write` is the pre-existing row; `workspace_edit` (the
    // replace-only variant, this slice) shares its permit and its effect —
    // the loop's gate keys on the effect, not the tool name.
    let tools = DatabaseTools::definitions(true, true, false, true, true);
    let edit = tools
        .iter()
        .find(|tool| tool.name == "workspace_edit")
        .expect("workspace_edit must be registered when workspace writes are permitted");
    assert_eq!(
        edit.effect.local_state,
        LocalStateEffect::WriteWorkspace,
        "workspace_edit must declare local_state == WriteWorkspace"
    );
    let write = tools
        .iter()
        .find(|tool| tool.name == "workspace_write")
        .expect("workspace_write must be registered when workspace writes are permitted");
    assert_eq!(
        write.effect.local_state,
        LocalStateEffect::WriteWorkspace,
        "workspace_write must keep declaring local_state == WriteWorkspace"
    );

    // No agent tool writes local state any more. Phase F retired
    // `contract_propose`: the harness extracts proposals post-turn from a bounded
    // turn record, so learning no longer depends on the model volunteering a call.
    // Asserting the tool is *absent* is the point — if it reappears, two paths to
    // the same write exist again and the model has to choose between them.
    let tools = DatabaseTools::definitions(true, true, true, false, true);
    assert!(
        !tools.iter().any(|tool| tool.name == "contract_propose"),
        "contract_propose is retired; the harness owns proposals"
    );
    assert!(
        !tools
            .iter()
            .any(|tool| tool.effect.local_state == LocalStateEffect::WriteCandidate),
        "no agent tool writes local state once the harness owns extraction"
    );
}
