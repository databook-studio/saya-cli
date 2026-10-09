/// A `run_command` completion whose summary parses to an exit code becomes a
/// check: 0 passes, anything else fails; the label is the redacted one-line
/// argv, bounded at the contract's character cap; run_program, unparsable
/// summaries, and denied calls record nothing.
#[test]
fn a_run_command_exit_code_becomes_a_check() {
    let mut observations = SessionObservations::default();
    observations.absorb(
        &output(
            "ran",
            Vec::new(),
            vec![
                requested_run_command(serde_json::json!({
                    "program": "cargo",
                    "args": ["test", "--token=abc123secret"],
                })),
                completed_run_command("cargo exited 0"),
                requested_run_command(serde_json::json!({"program": "cargo", "args": ["build"]})),
                completed_run_command("cargo exited 101"),
                completed_run_command("pytest exited 0"),
                AgentEvent::ToolCompleted {
                    name: "run_program".into(),
                    summary: "program ran".into(),
                },
                completed_run_command("cargo vanished"),
                AgentEvent::ToolDenied {
                    name: "run_command".into(),
                    reason: "not allowed".into(),
                },
            ],
        ),
        NOW,
    );
    let labels: Vec<&str> = observations
        .checks
        .iter()
        .map(|check| check.label.as_str())
        .collect();
    assert_eq!(
        labels,
        vec!["cargo test --token=[redacted]", "cargo build", "pytest"],
        "exactly the parsable run_command completions become checks: {labels:?}"
    );
    assert_eq!(observations.checks[0].outcome, CheckOutcome::Passed);
    assert_eq!(observations.checks[1].outcome, CheckOutcome::Failed);
    assert_eq!(observations.checks[2].outcome, CheckOutcome::Passed);
    for check in &observations.checks {
        assert_eq!(check.tool, "run_command");
        assert_eq!(check.observed_unix_ms, NOW);
        assert!(
            check.label.chars().count() <= 200,
            "label over the bound: {:?}",
            check.label
        );
    }
    assert!(
        !observations.checks[0].label.contains("abc123secret"),
        "the argv secret survived redaction: {:?}",
        observations.checks[0].label
    );
    assert!(observations.files.is_empty());
}

/// A long argv label truncates at exactly the bound, on a character
/// boundary — multibyte arguments cannot panic or split a character.
#[test]
fn a_long_argv_label_truncates_at_the_character_bound() {
    let mut observations = SessionObservations::default();
    observations.absorb(
        &output(
            "ran",
            Vec::new(),
            vec![
                requested_run_command(serde_json::json!({
                    "program": "echo",
                    "args": ["é".repeat(300)],
                })),
                completed_run_command("echo exited 0"),
            ],
        ),
        NOW,
    );
    assert_eq!(observations.checks.len(), 1);
    assert_eq!(observations.checks[0].label.chars().count(), 200);
}
