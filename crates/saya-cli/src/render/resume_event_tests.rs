use super::*;

#[test]
fn resume_uncertainty_keeps_bounded_machine_codes_and_text_labels() {
    let event = TerminalEvent::ResumeUncertain {
        run_id: "run-1".into(),
        step: 2,
        goal: "publish the report".into(),
        effects: vec![ResumeEffectCode::WorkspaceWrite, ResumeEffectCode::Fetch],
    };

    let json = render_event(&event, RenderFormat::Ndjson);
    let line: serde_json::Value = serde_json::from_str(&json.stderr).unwrap();
    assert_eq!(line["event"], "resume_uncertain");
    assert_eq!(line["run_id"], "run-1");
    assert_eq!(line["step"], 2);
    assert_eq!(
        line["effects"],
        serde_json::json!(["workspace_write", "fetch"])
    );
    assert!(json.stdout.is_empty());

    let text = render_event(&event, RenderFormat::Text);
    assert!(text.stderr.contains("step 2"));
    assert!(text.stderr.contains("workspace mutation"));
    assert!(text.stderr.contains("fetch or download"));
}
