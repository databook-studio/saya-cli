/// The inverse pin's first half: `fetch:` is wired, so `--allow
/// fetch:https+example.com` approves the scope — the deletion of its
/// refusal entry alone proves nothing, this does. The second half (the
/// fetch tools in the universe of the steps that asked) lives beside the
/// toolset builder's tests below.
#[test]
fn fetch_is_wired_and_still_approves() {
    use super::scopes::parse;

    let Ok(approved) = parse(
        &["fetch:https+example.com".to_string()],
        super::scopes::Surface::Run,
    ) else {
        panic!("the wired scope must approve");
    };
    let fetch = approved
        .capabilities
        .fetch
        .expect("the fetch scope must be approved");
    assert_eq!(
        fetch
            .destinations
            .iter()
            .map(|destination| (destination.scheme.as_str(), destination.host.as_str()))
            .collect::<Vec<_>>(),
        vec![("https", "example.com")],
        "the declared destinations are the approved ones"
    );
    assert!(!approved.capabilities.workspace_write);
    assert!(!approved.capabilities.scratch);
    assert!(approved.capabilities.runner.is_none());
    assert!(approved.capabilities.endpoints.as_map().is_empty());
}

/// The inverse pin's second half: the fetch tools are in the universe of
/// the steps that asked for fetch — appended after the database universe —
/// while a sibling step in the same plan that did not ask never sees them,
/// in either its definitions or its executor. The run-level admission alone
/// (the run approved fetch) must not leak the tools into the non-asking
/// step.
#[tokio::test]
async fn the_fetch_tools_are_in_the_universe_of_the_steps_that_asked_for_it() {
    let database = Arc::new(DatabaseTools::new(None, 100, true));
    let workspace = workspace("fetch-universe");
    let (_calls, fetch) = run_fetch(b"irrelevant");

    let mut fetch_caps = Capabilities::default();
    fetch_caps.fetch = Some(
        saya_types::FetchScope::new(vec![
            saya_types::Destination::new("https", "files.example.org").unwrap(),
        ])
        .expect("shaped"),
    );
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
            step("pull the corpus", fetch_caps.clone()),
            step("read only", Capabilities::default()),
        ],
    );

    // The asking step: both fetch tools, appended last, and no workspace
    // write tool — fetch alone is not the workspace-write scope.
    assert_eq!(
        names(&built, 0),
        [OPEN_GATE_WITH_CHART, FETCH_TAIL].concat(),
        "the fetch-asking step's universe must carry both fetch tools"
    );
    assert!(
        !names(&built, 0).contains(&"workspace_write".to_string()),
        "fetch alone must not carry the workspace-write tool"
    );
    // The non-asking step: byte-identical to the unchanged universe.
    assert_eq!(
        names(&built, 1),
        OPEN_GATE,
        "a step that did not ask for fetch must never see the tools"
    );

    // And the executor narrows with the definitions: the non-asking step's
    // composite refuses both names even though the run admitted the scope.
    for name in ["http_fetch", "http_download"] {
        let error = built[1]
            .executor
            .execute(name, serde_json::json!({}))
            .await
            .expect_err("the non-asking step has no fetch member behind its composite");
        assert_eq!(
            error,
            ToolError::UnsupportedTool,
            "`{name}` must be unknown to the step that did not ask"
        );
    }
}

/// The slice's headline gate: `--allow fetch:...` alone — without
/// `workspace-write` — downloads through `http_download` for real, through
/// the same composite the episodes dispatch through. The permit union
/// carries the write permit for a fetch-only step (pinned in the harness's
/// brief tests) and the egress permit carries the loop's guard down.
#[tokio::test]
async fn a_download_runs_through_the_composite_with_only_the_fetch_scope() {
    let database = Arc::new(DatabaseTools::new(None, 100, true));
    let workspace = workspace("download");
    let content = b"the corpus bytes".to_vec();
    let (_calls, fetch) = run_fetch(&content);

    let mut fetch_caps = Capabilities::default();
    fetch_caps.fetch = Some(
        saya_types::FetchScope::new(vec![
            saya_types::Destination::new("https", "files.example.org").unwrap(),
        ])
        .expect("shaped"),
    );
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
        &[step("pull the corpus", fetch_caps.clone())],
    );

    assert_eq!(
        names(&built, 0),
        [OPEN_GATE_WITH_CHART, FETCH_TAIL].concat(),
        "the step's universe must carry both fetch tools and nothing write-shaped besides"
    );

    let result = built[0]
        .executor
        .execute(
            "http_download",
            serde_json::json!({
                "url": "https://files.example.org/corpus.bin",
                "destination": "downloads/corpus.bin"
            }),
        )
        .await
        .expect("the download must run through the composite with fetch alone");
    let outcome = result.as_object().expect("the download's metadata");
    assert_eq!(outcome["bytes"], content.len() as u64);
    assert!(
        outcome["sha256"]
            .as_str()
            .is_some_and(|digest| digest.len() == 64),
        "the digest the resume verifies against: {outcome:?}"
    );
    let landed = std::fs::read(std::env::temp_dir().join(format!(
        "saya-run-tools-ws-download-{}/downloads/corpus.bin",
        std::process::id()
    )))
    .expect("the file landed in the run workspace");
    assert_eq!(landed, content, "the bytes on disk are the served bytes");
    let _ = fs::remove_dir_all(
        std::env::temp_dir().join(format!("saya-run-tools-ws-download-{}", std::process::id())),
    );
}
