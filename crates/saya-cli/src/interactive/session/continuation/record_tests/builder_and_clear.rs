/// The collector stays inside the contract's fact bounds: 64 files with the
/// latest touch winning and the oldest evicted, 32 checks likewise.
#[test]
fn observations_are_bounded_and_latest_wins() {
    let mut observations = SessionObservations::default();
    let mut files: Vec<ToolMetadata> = (0..70)
        .map(|index| {
            tool(
                "workspace_write",
                "completed",
                &format!(r#"{{"path":"src/f{index}.rs"}}"#),
            )
        })
        .collect();
    files.push(tool(
        "workspace_write",
        "completed",
        r#"{"path":"src/f0.rs"}"#,
    ));
    observations.absorb(&output("done", files, Vec::new()), NOW);
    assert_eq!(observations.files.len(), 64, "the file bound holds");
    assert_eq!(
        observations.files[63].path, "src/f0.rs",
        "latest touch wins"
    );
    assert_eq!(observations.files[0].path, "src/f7.rs", "oldest evicted");

    let mut checks_only = SessionObservations::default();
    let events: Vec<AgentEvent> = (0..40)
        .flat_map(|index| {
            [
                requested_run_command(serde_json::json!({"program": format!("p{index}")})),
                completed_run_command(format!("p{index} exited 0")),
            ]
        })
        .collect();
    checks_only.absorb(&output("done", Vec::new(), events), NOW);
    assert_eq!(checks_only.checks.len(), 32, "the check bound holds");
    assert_eq!(checks_only.checks[0].label, "p8", "oldest evicted");
    assert_eq!(checks_only.checks[31].label, "p39");
}

/// The builder's output is deterministic, carries the resolver's digest for
/// every file (malformed resolver output drops to none), and validates as a
/// version-1 continuation record.
#[test]
fn the_builder_output_validates_and_is_deterministic() {
    let mut observations = SessionObservations::default();
    observations.absorb(
        &output(
            "done",
            vec![tool(
                "workspace_write",
                "completed",
                r#"{"path":"src/x.rs"}"#,
            )],
            vec![
                requested_run_command(serde_json::json!({"program": "cargo", "args": ["test"]})),
                completed_run_command("cargo exited 0"),
            ],
        ),
        NOW,
    );
    let turns = vec![turn("hello", "hi")];
    let resolver = |path: &str| (path == "src/x.rs").then(|| "a".repeat(64));
    let facts = build_continuation_facts(
        &turns,
        Some("/workspace"),
        Some("profile"),
        &observations,
        &resolver,
    );
    assert_eq!(facts.source.through_turn, 1);
    assert_eq!(facts.source.workspace_root.as_deref(), Some("/workspace"));
    assert_eq!(facts.source.profile.as_deref(), Some("profile"));
    let expected_digest = "a".repeat(64);
    assert_eq!(
        facts.files[0].sha256.as_deref(),
        Some(expected_digest.as_str())
    );
    let record = SessionContinuationV1 {
        version: CONTINUATION_VERSION,
        source: facts.source.clone(),
        narrative: None,
        user_notes: None,
        files: facts.files.clone(),
        checks: facts.checks.clone(),
    };
    record
        .validate()
        .expect("the built record validates against the contract");
    let again = build_continuation_facts(
        &turns,
        Some("/workspace"),
        Some("profile"),
        &observations,
        &resolver,
    );
    assert_eq!(facts, again, "the builder is deterministic");
    let malformed = build_continuation_facts(&turns, None, None, &observations, &|_| {
        Some("not-a-digest".into())
    });
    assert_eq!(
        malformed.files[0].sha256, None,
        "malformed resolver output drops to none"
    );
}

/// The source boundary is positional over the persisted turns and its digest
/// is over the REDACTED data: the ordinal is the turn count, any text change
/// moves the digest, and two turns differing only by a secret hash alike.
#[test]
fn the_source_boundary_tracks_the_persisted_turns() {
    let observations = SessionObservations::default();
    let no_digest = &|_: &str| None;
    let turns = vec![
        turn("run the query", "done"),
        turn("now export", "exported"),
    ];
    let facts = build_continuation_facts(&turns, None, None, &observations, no_digest);
    assert_eq!(
        facts.source.through_turn, 2,
        "the ordinal is the turn count"
    );
    let one_turn = vec![turn("run the query", "done")];
    let one = build_continuation_facts(&one_turn, None, None, &observations, no_digest);
    assert_eq!(one.source.through_turn, 1);
    assert_ne!(
        facts.source.source_sha256, one.source.source_sha256,
        "a different turn count is a different source"
    );
    let changed = vec![
        turn("run the query", "different"),
        turn("now export", "exported"),
    ];
    let changed = build_continuation_facts(&changed, None, None, &observations, no_digest);
    assert_ne!(
        facts.source.source_sha256, changed.source.source_sha256,
        "a changed turn moves the digest"
    );
    let secret_a = vec![turn("connect with password=aaa", "ok")];
    let secret_b = vec![turn("connect with password=bbb", "ok")];
    assert_eq!(
        build_continuation_facts(&secret_a, None, None, &observations, no_digest)
            .source
            .source_sha256,
        build_continuation_facts(&secret_b, None, None, &observations, no_digest)
            .source
            .source_sha256,
        "the digest is over redacted data, so equal redactions hash alike"
    );
}

/// `/clear` empties the collector with the conversation, so observations
/// from cleared turns can never become facts of the next one.
#[test]
fn clear_resets_the_observations() {
    let mut state = SessionState::new("s1", None, "m");
    state.record_agent_output(
        "write it",
        &output(
            "done",
            vec![tool(
                "workspace_write",
                "completed",
                r#"{"path":"src/x.rs"}"#,
            )],
            Vec::new(),
        ),
    );
    assert!(
        !state.observations.files.is_empty(),
        "the turn's observation is recorded before the clear"
    );
    state.apply(SlashCommand::Clear, &[]);
    assert!(
        state.observations.files.is_empty() && state.observations.checks.is_empty(),
        "/clear must empty the collector: {:?}",
        state.observations
    );
}
