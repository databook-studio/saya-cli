/// The expected names are stated here, not derived from the builder's own
/// construction, so a drift is a diff in this test rather than a silent
/// universe change. Read-only and write-approving steps keep the exact
/// universe the single run-level one built; a scratch-asking step gets
/// `scratch_sql` appended — and a step that did not ask, in the same plan,
/// never sees it.
#[test]
fn every_step_s_definitions_follow_the_step_s_capabilities() {
    let database = Arc::new(DatabaseTools::new(None, 100, true));
    let workspace = workspace("defs");

    // Read-only steps: the unchanged universe, byte-identical across steps.
    let built = toolsets(
        ToolsetInputs {
            database: &database,
            scratch: None,
            fetch: None,
            runner: None,
            workspace: &workspace,
            allow_query_data: true,
            cancellation: &cancellation(),
        },
        &[
            step("read the schema", Capabilities::default()),
            step("summarize", Capabilities::default()),
        ],
    );
    for step_index in 0..2 {
        assert_eq!(
            names(&built, step_index),
            OPEN_GATE,
            "step {step_index}'s universe appeared, lost, or reordered a definition"
        );
        // The write filter still keys on the approved scopes, and with none
        // approved every tool is read-shaped or gate-shaped exactly as
        // before: no definition may carry the write effect.
        for definition in &built[step_index].definitions {
            assert_ne!(
                definition.effect.local_state,
                LocalStateEffect::WriteWorkspace,
                "{} must be hidden from a run without a write-shaped scope",
                definition.name
            );
        }
    }
    // Byte-identical across steps — the serialized definitions agree, not
    // merely the names.
    assert_eq!(
        serde_json::to_string(&built[0].definitions).unwrap(),
        serde_json::to_string(&built[1].definitions).unwrap(),
        "steps with the same capabilities must carry the same universe"
    );

    let built = toolsets(
        ToolsetInputs {
            database: &database,
            scratch: None,
            fetch: None,
            runner: None,
            workspace: &workspace,
            allow_query_data: false,
            cancellation: &cancellation(),
        },
        &[step("closed gate", Capabilities::default())],
    );
    assert_eq!(names(&built, 0), CLOSED_GATE);

    let mut write_scopes = Capabilities::default();
    write_scopes.workspace_write = true;
    let built = toolsets(
        ToolsetInputs {
            database: &database,
            scratch: None,
            fetch: None,
            runner: None,
            workspace: &workspace,
            allow_query_data: true,
            cancellation: &cancellation(),
        },
        &[step("write files", write_scopes)],
    );
    assert_eq!(
        names(&built, 0),
        [
            "schema_discovery",
            "request_clarification",
            "workspace_read",
            "workspace_list",
            "glob",
            "grep",
            "workspace_edit",
            "workspace_write",
            "bounded_sql_query",
            "bounded_sql_query_all",
            "result_shape",
            "column_health",
            "join_check",
            "designate_answer",
        ],
        "the approved write tools must keep their place in the universe"
    );
}

/// A run step that approves no egress does not advertise `render_chart` —
/// the same advertised-but-always-refused defect the session surface had,
/// on the run surface — while a step that approved fetch or runner does.
#[test]
fn a_step_without_egress_hides_render_chart_and_an_egress_step_advertises_it() {
    use saya_types::{Destination, FetchScope};
    let database = Arc::new(DatabaseTools::new(None, 100, true));
    let workspace = workspace("chart-egress");
    let (_calls, fetch) = run_fetch(b"irrelevant");

    let mut fetch_caps = Capabilities::default();
    fetch_caps.fetch = Some(
        FetchScope::new(vec![
            Destination::new("https", "files.example.org").unwrap(),
        ])
        .expect("shaped"),
    );
    let mut fetch_write_caps = fetch_caps.clone();
    fetch_write_caps.workspace_write = true;
    let built = toolsets(
        ToolsetInputs {
            database: &database,
            scratch: None,
            fetch: Some(&fetch),
            runner: None,
            workspace: &workspace,
            allow_query_data: true,
            cancellation: &cancellation(),
        },
        &[
            step("read only", Capabilities::default()),
            step("pull the corpus", fetch_caps),
            step("pull and save", fetch_write_caps),
        ],
    );
    assert!(
        !names(&built, 0).contains(&"render_chart".to_string()),
        "a step that approved no egress must not advertise render_chart: {:?}",
        names(&built, 0)
    );
    assert_eq!(
        names(&built, 1),
        [OPEN_GATE_WITH_CHART, FETCH_TAIL].concat(),
        "the fetch-asking step keeps the chart with its fetch tools"
    );
    let chart = |index: usize| {
        built[index]
            .definitions
            .iter()
            .find(|tool| tool.name == "render_chart")
            .expect("egress step advertises the chart")
    };
    assert!(
        chart(1).parameters["properties"].get("save_to").is_none(),
        "egress without workspace-write must not offer chart saving"
    );
    assert_eq!(
        chart(1).effect.local_state,
        saya_agent::LocalStateEffect::None
    );
    assert!(
        chart(2).parameters["properties"].get("save_to").is_some(),
        "workspace-write permits chart save on that step"
    );
    assert_eq!(
        chart(2).effect.local_state,
        saya_agent::LocalStateEffect::WriteWorkspace
    );
}
