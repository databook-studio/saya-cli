#[test]
fn resume_process_emits_typed_uncertainty_without_calling_provider() {
    let env = test_root("resume-uncertain-process");
    let goal = "publish token=resume-secret";
    let (setup, _) = mock(vec![Scripted {
        body: plan_body_with_capabilities(&[(goal, serde_json::json!({"workspace_write": true}))]),
        delay_ms: 0,
    }]);
    let started = saya(
        &env,
        &[
            "--non-interactive",
            "run",
            "--allow",
            "workspace-write",
            "execute the risky step",
        ],
        &setup,
    );
    assert_eq!(started.status.code(), Some(6), "{}", stderr(&started));
    let listing = saya(&env, &["run", "list"], &setup);
    let id = newest_run_id(&listing);
    assert!(!id.is_empty());
    let journal = fs::read_to_string(env.runs.join(&id).join("events.ndjson")).unwrap();
    assert!(
        journal.contains("step_failed"),
        "the journal records the risky failure: {journal}"
    );

    let (resume_provider, calls) = counted_drop_server();
    let resumed = saya(
        &env,
        &["--format", "json", "run", "resume", &id],
        &resume_provider,
    );
    assert_eq!(resumed.status.code(), Some(6), "{}", stderr(&resumed));
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "refusal precedes provider work"
    );
    let stderr_text = stderr(&resumed);
    let event: serde_json::Value = serde_json::from_str(stderr_text.trim()).unwrap_or_else(|error| {
        panic!("typed refusal JSON must be on stderr; stdout={:?}, stderr={stderr_text:?}, error={error}", stdout(&resumed))
    });
    assert_eq!(event["event"], "resume_uncertain");
    assert_eq!(event["run_id"], id);
    assert_eq!(event["step"], 1);
    assert_eq!(event["effects"], serde_json::json!(["workspace_write"]));
    assert!(event["goal"].as_str().unwrap().contains("[redacted]"));
    assert!(!stderr_text.contains("resume-secret"));

    let (retry_provider, retried_call) = mock(vec![Scripted {
        body: sse("reconciled and retried"),
        delay_ms: 0,
    }]);
    let retried = saya(
        &env,
        &["run", "resume", &id, "--retry-incomplete"],
        &retry_provider,
    );
    assert_eq!(retried.status.code(), Some(0), "{}", stderr(&retried));
    assert!(retried_call.load(Ordering::SeqCst));
    let journal = fs::read_to_string(env.runs.join(&id).join("events.ndjson")).unwrap();
    let approvals = journal
        .lines()
        .filter(|line| line.contains("plan_approved"))
        .collect::<Vec<_>>();
    assert_eq!(approvals.len(), 1, "retry does not create a new grant");
    assert!(
        approvals[0].contains("workspace-write"),
        "the original journaled grant remains recorded: {}",
        approvals[0]
    );
    let approval: serde_json::Value = serde_json::from_str(approvals[0]).unwrap();
    assert_eq!(
        approval["scopes"],
        serde_json::json!(["workspace-write"]),
        "retry leaves the journaled approval unchanged"
    );
    let _ = fs::remove_dir_all(&env.root);
}
