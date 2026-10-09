/// A headless `saya run` without `--allow` refuses and exits non-zero, having
/// created no run directory. Assert the filesystem.
#[test]
fn headless_run_without_allow_refuses_before_anything_exists() {
    let env = test_root("refusal");
    // The provider is never contacted: the refusal precedes any assembly.
    let (address, _flag) = mock(Vec::new());
    let output = saya(
        &env,
        &["--non-interactive", "run", "answer the goal"],
        &address,
    );
    assert_eq!(
        output.status.code(),
        Some(2),
        "a headless run without --allow is a usage error; stderr: {}",
        stderr(&output)
    );
    assert!(
        stderr(&output).contains("--allow"),
        "the refusal must say what is missing: {}",
        stderr(&output)
    );
    let created = match fs::read_dir(&env.runs) {
        Ok(entries) => entries.count(),
        Err(_) => 0,
    };
    assert_eq!(
        created,
        0,
        "a refused run must create no run directory: {:?}",
        fs::read_dir(&env.runs).map(|entries| entries
            .flatten()
            .map(|e| e.path().to_owned())
            .collect::<Vec<PathBuf>>())
    );
    let _ = fs::remove_dir_all(&env.root);
}

/// An unknown scope name is a usage error (2), not a silently ignored scope.
#[test]
fn allow_with_an_unknown_scope_is_a_usage_error() {
    let env = test_root("unknown-scope");
    let (address, _flag) = mock(Vec::new());
    let output = saya(
        &env,
        &["--non-interactive", "run", "--allow", "nonsense", "a goal"],
        &address,
    );
    assert_eq!(
        output.status.code(),
        Some(2),
        "an unknown scope is a usage error; stderr: {}",
        stderr(&output)
    );
    assert!(
        stderr(&output).contains("unknown scope"),
        "the error must name the refused scope: {}",
        stderr(&output)
    );
    let _ = fs::remove_dir_all(&env.root);
}

/// A read-only run has an honest spelling: `--allow none` states the empty
/// scope set and the run starts — no write capability is approved, which the
/// persisted spec (the record a resume reloads) proves field by field, and
/// the run completes through its ordinary lifecycle.
#[test]
fn allow_none_starts_a_read_only_run_that_approves_no_write_capability() {
    let env = test_root("read-only");
    let (address, _ready) = mock(vec![
        Scripted {
            body: plan_body(&["survey the schema"]),
            delay_ms: 0,
        },
        Scripted {
            body: sse("survey complete"),
            delay_ms: 0,
        },
    ]);
    let started = saya(
        &env,
        &[
            "--non-interactive",
            "run",
            "--allow",
            "none",
            "survey the data quality",
        ],
        &address,
    );
    assert_eq!(
        started.status.code(),
        Some(0),
        "a read-only run must start and complete; stderr: {}",
        stderr(&started)
    );
    let mut entries = fs::read_dir(&env.runs)
        .expect("the runs root exists")
        .flatten()
        .map(|entry| entry.path())
        .collect::<Vec<PathBuf>>();
    assert_eq!(entries.len(), 1, "exactly one run exists: {entries:?}");
    let dir = entries.remove(0);
    let spec: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(dir.join("spec.json")).unwrap()).unwrap();
    let scopes = &spec["scopes"];
    assert_eq!(
        scopes["workspace_write"],
        serde_json::json!(false),
        "no write capability may be approved: {scopes}"
    );
    assert_eq!(
        scopes["scratch"],
        serde_json::json!(false),
        "no scratch capability may be approved: {scopes}"
    );
    assert!(
        scopes["fetch"].is_null(),
        "no fetch capability may be approved: {scopes}"
    );
    assert!(
        scopes["runner"].is_null(),
        "no runner capability may be approved: {scopes}"
    );
    assert_eq!(
        scopes["endpoints"],
        serde_json::json!({}),
        "no endpoint binding may be approved: {scopes}"
    );
    // And the run really ran: the empty approval is a start, not a refusal.
    let journal = fs::read_to_string(dir.join("events.ndjson")).unwrap();
    assert!(
        journal.contains("run_started") && journal.contains("\"completed\""),
        "the read-only run started and completed: {journal}"
    );
    // The journal states the empty approval explicitly — the payload a
    // resume re-grants from, so an edited `spec.json` can widen nothing
    // (U8): the `plan_approved` line carries `"scopes":[]`, the stated
    // empty set, never the pre-field fallback shape (a field-less line).
    let plan_approved = journal
        .lines()
        .find(|line| line.contains("\"plan_approved\""))
        .expect("the journal records the approval");
    assert!(
        plan_approved.contains("\"scopes\":[]"),
        "the none run's approval is journaled as the stated empty set: {plan_approved}"
    );
    let _ = fs::remove_dir_all(&env.root);
}

/// A plan whose steps ask for more than `--allow` granted is rejected with a
/// usage refusal that names the missing scope and its step — the model
/// cannot fix that by re-planning, and a headless run cannot ask. The
/// engine re-prompts to its bound of three, so the mock serves three plans.
#[test]
fn a_plan_asking_for_more_than_allow_granted_is_refused_naming_the_scope() {
    let env = test_root("needs-approval");
    let asking = plan_body_with_capabilities(&[(
        "seed the scratch database",
        serde_json::json!({"scratch": true}),
    )]);
    let (address, _ready) = mock(vec![
        Scripted {
            body: asking.clone(),
            delay_ms: 0,
        },
        Scripted {
            body: asking.clone(),
            delay_ms: 0,
        },
        Scripted {
            body: asking,
            delay_ms: 0,
        },
    ]);
    let output = saya(
        &env,
        &[
            "--non-interactive",
            "run",
            "--allow",
            "workspace-write",
            "seed and report",
        ],
        &address,
    );
    assert_eq!(
        output.status.code(),
        Some(2),
        "a plan asking beyond --allow is a usage refusal; stderr: {}",
        stderr(&output)
    );
    let message = stderr(&output);
    assert!(
        message.contains("scratch"),
        "the refusal must name the missing scope: {message}"
    );
    assert!(
        message.contains("step 0"),
        "the refusal must point at the asking step: {message}"
    );
    assert!(
        message.contains("--allow"),
        "the refusal must say the lever to widen: {message}"
    );
    let _ = fs::remove_dir_all(&env.root);
}
