/// A follow-up that re-designates different SQL keeps the first designation —
/// the event and the output cannot disagree about which query answers the
/// question. (Pins the first-designation-wins rule.)
#[tokio::test]
async fn a_re_designation_keeps_the_first_sql() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = Scripted {
        turns: vec![
            calls_only(vec![designation_call("d1", "SELECT count(*) FROM t")]),
            calls_only(vec![designation_call("d2", "SELECT count(*) FROM other")])
                .into_iter()
                .chain(std::iter::once(Ok(ProviderEvent::TextDelta(
                    "The answer is 42.".into(),
                ))))
                .collect(),
        ],
        requests: requests.clone(),
    };
    let output = run_agent(
        &provider,
        &ExecutedTools {
            names: Arc::new(Mutex::new(Vec::new())),
        },
        request(),
        definitions(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
    )
    .await
    .expect("the re-designation carries prose and ends the run");
    assert_eq!(output.answer, "The answer is 42.");
    assert_eq!(
        output.answer_sql.as_deref(),
        Some("SELECT count(*) FROM t"),
        "the first designation wins over the re-designation"
    );
    assert_eq!(
        designated_events(&output),
        vec!["SELECT count(*) FROM t"],
        "exactly one AnswerDesignated event, for the first designation"
    );
}

/// A designation recovered into a turn that hits the turn ceiling still
/// carries its SQL: the salvage that ends the run is built on the work the
/// designation already committed.
#[tokio::test]
async fn recovery_at_the_turn_ceiling_still_carries_the_designation() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let provider = Scripted {
        turns: vec![
            calls_only(vec![designation_call("d1", "SELECT count(*) FROM t")]),
            text_then_done("salvaged prose"),
        ],
        requests: requests.clone(),
    };
    let output = run_agent(
        &provider,
        &ExecutedTools {
            names: Arc::new(Mutex::new(Vec::new())),
        },
        request(),
        definitions(),
        AgentLimits {
            max_turns: Some(1),
            ..AgentLimits::default()
        },
        &AllowReadOnlyApproval,
    )
    .await
    .expect("the ceiling salvages the run instead of failing it");
    assert!(
        output.truncated,
        "the run was salvaged at the turn ceiling, not completed"
    );
    assert_eq!(
        output.answer_sql.as_deref(),
        Some("SELECT count(*) FROM t"),
        "the salvage carries the designated SQL"
    );
}
