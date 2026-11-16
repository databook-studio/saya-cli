/// The expected names are stated here, not derived from the builder's own
/// construction, so a drift is a diff in this test rather than a silent
/// universe change. Read-only and write-approving steps keep the exact
/// universe the single run-level one built; a fetch-asking step gets its
/// fetch pair appended — and a step that did not ask, in the same plan,
/// never sees it.
#[test]
fn every_step_s_definitions_follow_the_step_s_capabilities() {
    let database = Arc::new(DatabaseTools::new(None, 100, true));
    let workspace = workspace("defs");

    // Read-only steps: the unchanged universe, byte-identical across steps.
    let built = toolsets(
        ToolsetInputs {
            database: &database,
            fetch: None,
            shell: None,
            workspace: &workspace,
            allow_query_data: true,
            tools: &saya_config::ResolvedTools::default(),
            mcp: None,
            hooks: None,
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
            fetch: None,
            shell: None,
            workspace: &workspace,
            allow_query_data: false,
            tools: &saya_config::ResolvedTools::default(),
            mcp: None,
            hooks: None,
        },
        &[step("closed gate", Capabilities::default())],
    );
    assert_eq!(names(&built, 0), CLOSED_GATE);

    let mut write_scopes = Capabilities::default();
    write_scopes.workspace_write = true;
    let built = toolsets(
        ToolsetInputs {
            database: &database,
            fetch: None,
            shell: None,
            workspace: &workspace,
            allow_query_data: true,
            tools: &saya_config::ResolvedTools::default(),
            mcp: None,
            hooks: None,
        },
        &[step("write files", write_scopes)],
    );
    assert_eq!(
        names(&built, 0),
        [
            "schema_discovery",
            "request_clarification",
            "read",
            "glob",
            "grep",
            "edit",
            "write",
            "bounded_sql_query",
            "bounded_sql_query_all",
            "result_shape",
            "column_health",
            "join_check",
            "designate_answer",
        ],
        "the approved write tools must keep their place in the universe"
    );
    // `notebook_edit` is the one write tool this step does not carry: the
    // workspace it was built over holds no notebook (see
    // `tools_tests/notebook_presence.rs` for the same step where one exists).
}

/// The retired `render_chart` model tool is advertised on no run step —
/// whatever egress or workspace-write scope the step approved, it never
/// reappears.
#[test]
fn no_run_step_advertises_the_retired_render_chart() {
    use saya_types::{Destination, FetchScope};
    let database = Arc::new(DatabaseTools::new(None, 100, true));
    let workspace = workspace("chart-retired");
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
            fetch: Some(&fetch),
            shell: None,
            workspace: &workspace,
            allow_query_data: true,
            tools: &saya_config::ResolvedTools::default(),
            mcp: None,
            hooks: None,
        },
        &[
            step("read only", Capabilities::default()),
            step("pull the corpus", fetch_caps),
            step("pull and save", fetch_write_caps),
        ],
    );
    for step_index in 0..3 {
        let names = names(&built, step_index);
        assert!(
            !names.contains(&"render_chart".to_string()),
            "render_chart is retired; run step {step_index} still advertises it: {names:?}"
        );
    }
    assert_eq!(
        names(&built, 1),
        [OPEN_GATE, FETCH_TAIL].concat(),
        "the fetch-asking step carries the chart-less universe plus its fetch tools"
    );
}
