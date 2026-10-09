#[test]
fn coding_is_ready_in_a_git_worktree_with_zero_profiles() {
    let dir = temp_dir("worktree");
    std::fs::create_dir(dir.join(".git")).unwrap();
    let runtime = runtime_from(
        "[ai]\nprovider = \"ollama\"\nmodel = \"qwen2.5-coder:14b\"\n",
        "",
        BTreeMap::new(),
    );
    let readiness = readiness(&runtime, DoctorTask::Coding, &dir);
    // Zero profiles, ready AI, a worktree: coding is ready — the database is
    // NOT consulted for this task.
    assert!(readiness.ready, "{readiness:?}");
    let workspace = check_named(&readiness, "workspace");
    assert_eq!(workspace.status, CheckStatus::Ok);
    let canonical = std::fs::canonicalize(&dir).unwrap();
    assert_eq!(workspace.detail, canonical.display().to_string());
    assert_eq!(
        readiness
            .checks
            .iter()
            .map(|check| check.name.as_str())
            .collect::<Vec<_>>(),
        vec!["provider", "credentials", "endpoint", "workspace"]
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn coding_outside_any_worktree_fails_the_workspace_check() {
    let dir = temp_dir("no-worktree");
    if inside_a_worktree(&dir) {
        eprintln!(
            "skipping: {} unexpectedly sits inside a git worktree on this machine",
            dir.display()
        );
        return;
    }
    let runtime = runtime_from(
        "[ai]\nprovider = \"ollama\"\nmodel = \"qwen2.5-coder:14b\"\n",
        "",
        BTreeMap::new(),
    );
    let readiness = readiness(&runtime, DoctorTask::Coding, &dir);
    assert!(!readiness.ready, "{readiness:?}");
    let workspace = check_named(&readiness, "workspace");
    assert_eq!(workspace.status, CheckStatus::Fail);
    assert_eq!(
        workspace.detail,
        "no project directory here — run inside a git worktree"
    );
    let _ = std::fs::remove_dir_all(dir);
}

// --- serialization shape ---

#[test]
fn readiness_serializes_as_task_ready_checks_with_snake_case_statuses() {
    let dir = temp_dir("shape");
    std::fs::create_dir(dir.join(".git")).unwrap();
    let runtime = runtime_from(
        "[ai]\nprovider = \"ollama\"\nmodel = \"qwen2.5-coder:14b\"\n",
        "",
        BTreeMap::new(),
    );
    let readiness = readiness(&runtime, DoctorTask::Coding, &dir);
    let value: serde_json::Value =
        serde_json::from_str(&serde_json::to_string(&readiness).unwrap()).unwrap();
    assert_eq!(value["task"], "coding");
    assert_eq!(value["ready"], true);
    let checks = value["checks"].as_array().unwrap();
    assert_eq!(checks[0]["name"], "provider");
    assert_eq!(checks[0]["status"], "ok");
    assert!(checks[0]["detail"].is_string());
    assert_eq!(checks[1]["status"], "ok");
    let _ = std::fs::remove_dir_all(dir);
}
