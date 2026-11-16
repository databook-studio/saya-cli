/// An AI task without an api_key reference for a cloud provider is not
/// ready: exit 3, and the text report names the failing `credentials` check.
#[test]
fn doctor_task_ai_without_a_key_exits_three_and_names_the_credentials_check() {
    let root = test_root("doctor-task-ai-no-key");
    fs::create_dir_all(root.join("user-config/saya")).unwrap();
    fs::write(
        root.join("user-config/saya/config.toml"),
        "[ai]\nprovider = \"openai\"\nmodel = \"gpt-4o\"\n",
    )
    .unwrap();
    let output = saya_isolated(&root, &["config", "doctor", "--task", "ai"]);
    assert_eq!(output.status.code(), Some(3));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("task ai: NOT ready"), "{stdout}");
    assert!(stdout.contains("✗ credentials"), "{stdout}");
    let _ = fs::remove_dir_all(root);
}

/// NDJSON carries exactly one `doctor_task` event envelope on stdout: the
/// machine surface for a script that wants structured readiness.
#[test]
fn doctor_task_ai_ndjson_emits_one_doctor_task_event() {
    let root = test_root("doctor-task-ai-ndjson");
    fs::create_dir_all(root.join("user-config/saya")).unwrap();
    fs::write(
        root.join("user-config/saya/config.toml"),
        "[ai]\nprovider = \"openai\"\nmodel = \"gpt-4o\"\n",
    )
    .unwrap();
    let output = saya_isolated(
        &root,
        &["--format", "ndjson", "config", "doctor", "--task", "ai"],
    );
    assert_eq!(output.status.code(), Some(3));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 1, "exactly one stdout line: {stdout:?}");
    let value: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(value["event"], "doctor_task");
    assert_eq!(value["task"], "ai");
    assert_eq!(value["ready"], false);
    assert!(
        value["checks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|check| { check["name"] == "credentials" && check["status"] == "fail" })
    );
    let _ = fs::remove_dir_all(root);
}

/// `--task query` with nothing configured: no profile selected, exit 3.
#[test]
fn doctor_task_query_without_a_profile_exits_three() {
    let root = test_root("doctor-task-query-empty");
    let output = saya_isolated(&root, &["config", "doctor", "--task", "query"]);
    assert_eq!(output.status.code(), Some(3));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("task query: NOT ready"), "{stdout}");
    assert!(stdout.contains("✗ profile"), "{stdout}");
    let _ = fs::remove_dir_all(root);
}

/// Coding readiness is independent of the database: an isolated git worktree,
/// a ready local AI provider, and no connections file at all is exit 0.
#[test]
fn doctor_task_coding_is_ready_without_a_database_profile() {
    let root = test_root("doctor-task-coding-ready");
    fs::create_dir_all(root.join(".git")).unwrap();
    fs::create_dir_all(root.join("user-config/saya")).unwrap();
    fs::write(
        root.join("user-config/saya/config.toml"),
        "[ai]\nprovider = \"ollama\"\nmodel = \"qwen2.5-coder:14b\"\n",
    )
    .unwrap();
    let output = saya_isolated(&root, &["config", "doctor", "--task", "coding"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "coding must not require a database profile: stdout={:?} stderr={:?}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("task coding: ready"), "{stdout}");
    assert!(stdout.contains("✓ workspace"), "{stdout}");
    assert!(stdout.contains("✓ credentials"), "{stdout}");
    let _ = fs::remove_dir_all(root);
}

/// An unknown task value is a clap usage error (exit 2) that lists the valid
/// values, not a doctor run with a guessed task.
#[test]
fn doctor_task_rejects_unknown_values_with_a_usage_error() {
    let root = test_root("doctor-task-unknown");
    let output = saya_isolated(&root, &["config", "doctor", "--task", "nope"]);
    assert_eq!(output.status.code(), Some(2));
    let stderr_text = stderr(&output);
    assert!(
        stderr_text.contains("query")
            && stderr_text.contains("ai")
            && stderr_text.contains("coding"),
        "the usage error must list the valid values: {stderr_text}"
    );
    let _ = fs::remove_dir_all(root);
}
