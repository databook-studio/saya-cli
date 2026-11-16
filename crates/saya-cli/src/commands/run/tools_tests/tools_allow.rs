/// A run's per-step toolsets carry the one `[tools] allow` list exactly as a
/// session does: the step composes its surface as it always did and then keeps
/// only the listed names. The connection-tied tools stay — the list does not
/// govern them — and the step's own scope conditions still decide everything
/// they decided before, so a name in the list never grants a tool the step
/// would not otherwise have carried.
#[test]
fn a_stated_list_narrows_every_run_step() {
    use saya_types::{Destination, FetchScope};
    let database = Arc::new(DatabaseTools::new(None, 100, true));
    let workspace = workspace("tools-allow-run");
    let (_calls, fetch) = run_fetch(b"irrelevant");

    let mut fetch_caps = Capabilities::default();
    fetch_caps.fetch = Some(
        FetchScope::new(vec![Destination::new("https", "files.example.org").unwrap()])
            .expect("shaped"),
    );
    let allow = saya_config::ResolvedTools::from_stated(Some(&["read".to_string(), "glob".to_string()]))
        .expect("a valid list");
    let built = toolsets(
        ToolsetInputs {
            database: &database,
            fetch: Some(&fetch),
            shell: None,
            workspace: &workspace,
            allow_query_data: true,
            tools: &allow,
            mcp: None,
            hooks: None,
        },
        &[
            step("read only", Capabilities::default()),
            step("pull the corpus", fetch_caps),
        ],
    );
    for step_index in 0..2 {
        let names = names(&built, step_index);
        for tool in ["read", "glob"] {
            assert!(
                names.contains(&tool.to_string()),
                "step {step_index} keeps {tool}: {names:?}"
            );
        }
        for tool in ["write", "grep", "http_fetch"] {
            assert!(
                !names.contains(&tool.to_string()),
                "the list removed {tool} from step {step_index}: {names:?}"
            );
        }
        for tool in &names {
            assert!(
                allow.admits(tool) || saya_config::is_database_tool(tool),
                "step {step_index} advertises {tool}, which is neither in the list nor \
                 connection-tied: {names:?}"
            );
        }
    }
    // The fetch-asking step's own scope adds nothing: the list
    // removed the fetch pair. A scope condition can only ever withhold, so the
    // two agree — the step advertises what survives both.
    assert_eq!(
        names(&built, 0),
        names(&built, 1),
        "the scope tail cannot re-add a name the list removed"
    );
}

/// The list never grants what a step's own scope withholds: a step that
/// approved no fetch scope advertises no fetch tool even when the list names
/// one.
#[test]
fn a_stated_list_never_grants_a_scope_a_step_did_not_approve() {
    let database = Arc::new(DatabaseTools::new(None, 100, true));
    let workspace = workspace("tools-allow-scope");
    let (_calls, fetch) = run_fetch(b"irrelevant");
    let allow = saya_config::ResolvedTools::from_stated(Some(&[
        "read".to_string(),
        "http_fetch".to_string(),
        "http_download".to_string(),
    ]))
    .expect("a valid list");
    let built = toolsets(
        ToolsetInputs {
            database: &database,
            fetch: Some(&fetch),
            shell: None,
            workspace: &workspace,
            allow_query_data: true,
            tools: &allow,
            mcp: None,
            hooks: None,
        },
        &[step("read only", Capabilities::default())],
    );
    let names = names(&built, 0);
    assert!(
        !names.contains(&"http_fetch".to_string()),
        "the step approved no egress scope, so the list cannot add one: {names:?}"
    );
    assert!(names.contains(&"read".to_string()), "{names:?}");
}

/// The step's dispatch door follows its advertisement: a name the list
/// removed is refused as unknown through every entry point of the composite,
/// not merely left out of the definitions. `glob` is composed for a
/// read-only step, so the list is what takes it away.
#[tokio::test]
async fn a_run_step_refuses_a_name_the_list_removed() {
    use saya_agent::{CancellationToken, ToolExecutionContext};

    let database = Arc::new(DatabaseTools::new(None, 100, true));
    let workspace = workspace("tools-allow-refusal");
    let allow =
        saya_config::ResolvedTools::from_stated(Some(&["read".to_string()])).expect("a valid list");
    let built = toolsets(
        ToolsetInputs {
            database: &database,
            fetch: None,
            shell: None,
            workspace: &workspace,
            allow_query_data: true,
            tools: &allow,
            mcp: None,
            hooks: None,
        },
        &[step("read only", Capabilities::default())],
    );
    let names = names(&built, 0);
    assert!(
        names.contains(&"read".to_string()) && !names.contains(&"glob".to_string()),
        "the fixture narrows the step to the stated list: {names:?}"
    );

    let executor = &built[0].executor;
    let arguments = serde_json::json!({});
    let context = ToolExecutionContext {
        result_cap: 4096,
        cancellation: CancellationToken::new(),
    };
    for (door, refused) in [
        ("execute", executor.execute("glob", arguments.clone()).await),
        (
            "execute_with_result_cap",
            executor
                .execute_with_result_cap("glob", arguments.clone(), 4096)
                .await,
        ),
        (
            "execute_with_context",
            executor
                .execute_with_context("glob", arguments, context)
                .await,
        ),
    ] {
        let refused = refused.expect_err("a removed name must refuse, not be served");
        assert!(
            matches!(refused, ToolError::UnsupportedTool),
            "{door} must refuse a name the list removed: {refused:?}"
        );
        assert_eq!(
            refused.to_string(),
            "unsupported read-only tool",
            "{door} refuses with the same wording as an uncomposed member"
        );
    }
}
