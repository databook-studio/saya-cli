use super::*;

// ---------------------------------------------------------------------------
// launch `--allow` seeds every scope, never only `command:` — the relay
// bug: feeding the shared grammar-then-composition gate
// (`seed_launch_allow`) a pre-filtered `command:`-only copy of the launch's
// `--allow` tokens leaves every other scope type (`workspace-write`, `sql:`,
// `runner:`, `interpreter:`, `fetch:`) seeding nothing and saying
// nothing — a silent no-op. These pin that end to end, through the real
// launch path both surfaces share.
// ---------------------------------------------------------------------------

/// A launch `--allow workspace-write` seed lands in the grant store:
/// `/grants` on the one turn names it.
#[test]
fn a_launch_workspace_write_seed_is_granted() {
    let env = test_root("launch-workspace-write");
    let (address, _bodies) = mock("unused");
    let path = env.root.join("instruction.md");
    std::fs::write(&path, "/grants\n").unwrap();
    // A dedicated subdir, sibling to `env.sessions` — `--workspace` must
    // not contain the session store (a separate, unrelated containment
    // guard refuses that).
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();

    let mut args = base_args(&env);
    args.push("--workspace".into());
    args.push(workspace.to_str().unwrap().into());
    args.push("--allow".into());
    args.push("workspace-write".into());
    args.push("--turn-file".into());
    args.push(path.to_str().unwrap().into());
    let output = saya(
        &env,
        &address,
        &args.iter().map(String::as_str).collect::<Vec<_>>(),
    )
    .output()
    .unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "a carriable launch seed starts the session; stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("workspace-write"),
        "the launch seed is granted: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let _ = std::fs::remove_dir_all(&env.root);
}

/// A launch `--allow sql:<connection>` seed lands in the grant store —
/// `sql:` gates nothing at composition (`allow_refusal` has no arm for it),
/// so no workspace binding is needed for it to seed.
#[test]
fn a_launch_sql_seed_is_granted() {
    let env = test_root("launch-sql");
    let (address, _bodies) = mock("unused");
    let path = env.root.join("instruction.md");
    std::fs::write(&path, "/grants\n").unwrap();

    let mut args = base_args(&env);
    args.push("--allow".into());
    args.push("sql:local".into());
    args.push("--turn-file".into());
    args.push(path.to_str().unwrap().into());
    let output = saya(
        &env,
        &address,
        &args.iter().map(String::as_str).collect::<Vec<_>>(),
    )
    .output()
    .unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("sql:local"),
        "the launch seed is granted: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let _ = std::fs::remove_dir_all(&env.root);
}

/// A launch `--allow runner:report` seed is a usage error: the runner and
/// interpreter grant families do not exist, so the token is refused at
/// the grammar with one wording that names the replacement, and the process
/// refuses to start rather than silently dropping the seed. The refusal no
/// longer turns on whether a runner is composed — no session composes one
/// any more — so this pins the removal wording itself.
#[test]
fn a_launch_runner_seed_without_a_runner_is_a_usage_error() {
    let env = test_root("launch-runner-no-runner");
    let (address, _bodies) = mock("unused");
    let path = env.root.join("instruction.md");
    std::fs::write(&path, "/grants\n").unwrap();

    let mut args = base_args(&env);
    args.push("--allow".into());
    args.push("runner:report".into());
    args.push("--turn-file".into());
    args.push(path.to_str().unwrap().into());
    let output = saya(
        &env,
        &address,
        &args.iter().map(String::as_str).collect::<Vec<_>>(),
    )
    .output()
    .unwrap();
    assert_ne!(
        output.status.code(),
        Some(0),
        "a seed the composition cannot carry refuses to start: stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        stderr.contains("invalid --allow seed")
            && stderr.contains("runner and interpreter grants were removed"),
        "the refusal states the removal: {stderr}"
    );
    assert!(
        stderr.contains("use the shell tool"),
        "the refusal names the replacement: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&env.root);
}

/// A launch `--allow runner:python3` seed is refused too: the whole runner
/// and interpreter grant family does not exist, so even a well-known
/// interpreter payload carries nothing. The refusal is the grammar's —
/// before composition is consulted — and names the replacement, never
/// silently dropping the seed.
#[test]
fn a_launch_grammar_error_is_a_usage_error() {
    let env = test_root("launch-grammar-error");
    let (address, _bodies) = mock("unused");
    let path = env.root.join("instruction.md");
    std::fs::write(&path, "/grants\n").unwrap();

    let mut args = base_args(&env);
    args.push("--allow".into());
    args.push("runner:python3".into());
    args.push("--turn-file".into());
    args.push(path.to_str().unwrap().into());
    let output = saya(
        &env,
        &address,
        &args.iter().map(String::as_str).collect::<Vec<_>>(),
    )
    .output()
    .unwrap();
    assert_ne!(
        output.status.code(),
        Some(0),
        "a grammar-refused seed refuses to start: stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        stderr.contains("invalid --allow seed")
            && stderr.contains("runner and interpreter grants were removed"),
        "the refusal states the removal: {stderr}"
    );
    assert!(
        stderr.contains("use the shell tool"),
        "the refusal names the replacement: {stderr}"
    );
    let _ = std::fs::remove_dir_all(&env.root);
}

/// Regression: a launch `--allow command:git` seed still works after the
/// relay fix — granted exactly once, journalled exactly once (the previous
/// relay granted `command:` tokens through two separate paths).
#[cfg(unix)]
#[test]
fn a_launch_command_seed_still_works() {
    let env = test_root("launch-command");
    let (address, _bodies) = mock("unused");
    let path = env.root.join("instruction.md");
    std::fs::write(&path, "/grants\n").unwrap();
    // A dedicated subdir, sibling to `env.sessions` — `--workspace` must
    // not contain the session store (a separate, unrelated containment
    // guard refuses that).
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();

    let mut args = base_args(&env);
    args.push("--workspace".into());
    args.push(workspace.to_str().unwrap().into());
    args.push("--allow".into());
    args.push("command:git".into());
    args.push("--turn-file".into());
    args.push(path.to_str().unwrap().into());
    let output = saya(
        &env,
        &address,
        &args.iter().map(String::as_str).collect::<Vec<_>>(),
    )
    .output()
    .unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("command:git"),
        "the launch seed is granted: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    // Exactly one session directory exists (a fresh session per run); its
    // journal carries exactly one `Granted` line for the token, never two.
    let mut sessions = std::fs::read_dir(&env.sessions)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_dir());
    let session_dir = sessions.next().expect("the run created one session dir");
    assert!(
        sessions.next().is_none(),
        "exactly one session directory from this run"
    );
    let journal = std::fs::read_to_string(session_dir.path().join("journal.ndjson")).unwrap();
    let granted_lines = journal
        .lines()
        .filter(|line| line.contains("command:git") && line.contains("session-granted"))
        .count();
    assert_eq!(
        granted_lines, 1,
        "the token is journalled exactly once, never doubled: {journal}"
    );
    let _ = std::fs::remove_dir_all(&env.root);
}
