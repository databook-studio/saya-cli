/// The probe gates every door, and its refusal is *said* (DESIGN §2, test
/// 15): a config that declared a program dir and an allow can compose no
/// runner for exactly one reason — the probe did not prove this host — and
/// the absence lands on the notice seam, referenced by the bypass activation
/// line. On a proven host the positive control holds: the runner composes,
/// the notice stays silent, and `run_program` is advertised.
#[test]
fn bypass_composes_no_runner_where_the_probe_refuses_and_says_so() {
    use crate::interactive::session_runner::PROBE_REFUSED_NOTICE;
    let project = worktree("probe-refusal");
    let state = temp_dir("probe-refusal-state");
    let tools = temp_dir("probe-refusal-tools");
    let runtime = session_runtime(Some((vec!["bench".to_string()], Some(tools))));
    let universe = compose(&runtime, &project, &state);
    match universe.runner.as_ref() {
        Some(_) => {
            // Positive control (a proven host): the runner is there, nothing
            // is said, and bypass advertises the tool.
            assert!(
                !universe.probe_refused && universe.notice.is_none(),
                "a proven probe is silent: {:?}",
                universe.notice
            );
            assert!(
                advertised(&universe, AgentMode::Build, ApprovalPolicy::Bypass, false)
                    .contains(&"run_program".to_string()),
                "bypass advertises the proven runner with no prompt surface"
            );
        }
        None => {
            // The config declared program_dir and allow, and the placement
            // guard passed (composition did not Err) — so the only possible
            // cause of an absent runner is the refused probe. It must be
            // said, and run_program must be absent from every mode's list.
            let notice = universe
                .notice
                .as_deref()
                .expect("a refused probe is said, never silent");
            if cfg!(windows) {
                assert!(notice.contains(PROBE_REFUSED_NOTICE), "{notice:?}");
                assert!(
                    notice.contains("Host commands are unavailable on Windows"),
                    "{notice:?}"
                );
            } else {
                assert_eq!(notice, PROBE_REFUSED_NOTICE);
            }
            assert!(universe.probe_refused, "the activation line's fact rides");
            assert!(
                !advertised(&universe, AgentMode::Build, ApprovalPolicy::Bypass, false)
                    .contains(&"run_program".to_string()),
                "no runner, no run_program advertisement — under bypass like any mode"
            );
        }
    }
    // The words the refusal emits are the design's line, whatever host this
    // test runs on.
    assert!(
        PROBE_REFUSED_NOTICE.contains("run_program is unavailable")
            && PROBE_REFUSED_NOTICE.contains("the sandbox probe did not prove this host"),
        "the notice names the probe: {PROBE_REFUSED_NOTICE}"
    );
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

// ---------------------------------------------------------------------------
// the SQL safety layer is untouchable under bypass
// ---------------------------------------------------------------------------

/// The SQL safety layer is not an approval question, so bypass does not move
/// it (DESIGN §4, test 14): a write statement through the query tools still
/// refuses under a bypass policy — and the refusal is the safety layer's own
/// words, not the approval engine's, because the engine resolved `Allow` and
/// handed the call to the tool. A real DuckDB connector backs the registry,
/// so the statement meets the actual `prepare_*` gate.
#[tokio::test]
async fn bypass_leaves_the_sql_safety_layer_untouched() {
    use saya_connectors::{ConnectorOptions, DuckDbConnector};
    let project = worktree("safety-layer");
    let state = temp_dir("safety-layer-state");
    let universe = compose(&session_runtime(None), &project, &state);
    let connector = DuckDbConnector::open(":memory:", false, ConnectorOptions::default())
        .await
        .expect("an in-memory duckdb opens");
    let database = Arc::new(crate::agent::tools::DatabaseTools::new(
        Some(Box::new(connector)),
        100,
        true,
    ));
    // The engine half: under bypass the write-shaped SQL call is *allowed* —
    // the refusal cannot come from approval.
    let sql = universe
        .definitions(
            saya_agent::AgentMode::Build,
            ApprovalPolicy::Bypass,
            false,
            true,
            false,
            false,
        )
        .iter()
        .find(|definition| definition.name == "bounded_sql_query")
        .expect("the read-shaped SQL tools are always advertised")
        .clone();
    assert_eq!(
        saya_agent::SessionPolicy::new(ApprovalPolicy::Bypass).resolve(&sql.effect, None),
        saya_agent::ApprovalDecision::Allow,
        "bypass allows the SQL call: the safety layer is the next line of defence, not approval"
    );
    let executor = universe.executor(database, &CancellationToken::new(), false);
    let refused = executor
        .execute(
            "bounded_sql_query",
            serde_json::json!({"sql": "DROP TABLE users"}),
        )
        .await
        .expect_err("a write statement still refuses under bypass");
    let refusal = format!("{refused:?}");
    assert!(
        refusal.contains("read-only safety policy")
            || refusal.contains("not parseable as one read-only statement"),
        "the refusal is the safety layer's own: {refusal}"
    );
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}
