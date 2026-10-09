#[test]
fn render_chart_requires_approval() {
    let tools = DatabaseTools::definitions(true, false, false, false, true);
    let chart_tool = tools
        .iter()
        .find(|tool| tool.name == "render_chart")
        .expect("render_chart definition exists");
    assert!(chart_tool.effect.requires_approval);
    assert!(chart_tool.effect.external_side_effect);
    assert!(!chart_tool.effect.database_data);
}

#[test]
fn render_chart_offers_workspace_save_only_with_write_permit() {
    let tools = DatabaseTools::definitions_with_chart_save(true, false, false, true, true, true);
    let chart_tool = tools
        .iter()
        .find(|tool| tool.name == "render_chart")
        .unwrap();
    assert_eq!(
        chart_tool.parameters["properties"]["save_to"]["type"],
        "string"
    );
    assert_eq!(
        chart_tool.effect.local_state,
        saya_agent::LocalStateEffect::WriteWorkspace
    );
    assert!(chart_tool.effect.requires_approval);
    assert!(chart_tool.effect.external_side_effect);

    let tools = DatabaseTools::definitions_with_chart_save(true, false, false, false, true, false);
    let chart_tool = tools
        .iter()
        .find(|tool| tool.name == "render_chart")
        .unwrap();
    assert!(chart_tool.parameters["properties"].get("save_to").is_none());
    assert_eq!(
        chart_tool.effect.local_state,
        saya_agent::LocalStateEffect::None
    );
}
