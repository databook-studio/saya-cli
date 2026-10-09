use super::*;

/// The flag is session-surface only: `saya ask` and `saya run` refuse it
/// rather than silently ignoring a stated intent.
#[test]
fn turn_file_is_refused_by_ask_and_run() {
    let env = test_root("refused");
    let (address, _bodies) = mock("unused");
    for subcommand in [vec!["ask", "hi"], vec!["run", "--allow", "none", "goal"]] {
        let mut args = vec![
            "--connections",
            env.connections.to_str().unwrap(),
            "--profile",
            "local",
        ];
        args.extend(subcommand.iter().copied());
        args.extend(["--turn-file", "instruction.md"]);
        let output = saya(&env, &address, &args).output().unwrap();
        assert_ne!(
            output.status.code(),
            Some(0),
            "{subcommand:?} must refuse --turn-file"
        );
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        assert!(
            stderr.contains("--turn-file"),
            "the refusal names the flag: {stderr}"
        );
    }
    let _ = std::fs::remove_dir_all(&env.root);
}

/// `/context` through the real headless binary: the workspace-root AGENTS.md
/// and the personal instructions.md are named with their receipts (size,
/// digest, truncation), and neither body text ever reaches stdout.
#[test]
fn a_turn_file_context_reports_sources_without_their_bodies() {
    const PROJECT_SENTINEL: &str = "SENTINEL-INSTRUCTION-BODY";
    const PERSONAL_SENTINEL: &str = "SENTINEL-PERSONAL-INSTRUCTION-BODY";
    let env = test_root("turn-file-context");
    let (address, _bodies) = mock("unused");
    let path = env.root.join("instruction.md");
    std::fs::write(&path, "/context\n").unwrap();
    // A dedicated subdir, sibling to `env.sessions` — `--workspace` must
    // not contain the session store (a separate, unrelated containment
    // guard refuses that).
    let workspace = env.root.join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let project_body = format!("project rules with {PROJECT_SENTINEL}\n");
    std::fs::write(workspace.join("AGENTS.md"), &project_body).unwrap();
    // The scratch config home is `env.root` (see `saya`), so the personal
    // instructions file lives at `<config home>/saya/instructions.md`.
    let personal_body = format!("personal rules with {PERSONAL_SENTINEL}\n");
    std::fs::create_dir_all(env.root.join("saya")).unwrap();
    std::fs::write(env.root.join("saya/instructions.md"), &personal_body).unwrap();

    let mut args = base_args(&env);
    args.push("--workspace".into());
    args.push(workspace.to_str().unwrap().into());
    args.push("--turn-file".into());
    args.push(path.to_str().unwrap().into());
    let output = saya(
        &env,
        &address,
        &args.iter().map(String::as_str).collect::<Vec<_>>(),
    )
    .output()
    .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert_eq!(
        output.status.code(),
        Some(0),
        "the /context turn completes: stdout: {stdout}\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains("AGENTS.md"),
        "the project source is named: {stdout}"
    );
    assert!(
        stdout.contains("instructions.md"),
        "the personal source is named: {stdout}"
    );
    assert!(
        stdout.contains(&format!("{} B file", project_body.len())),
        "the whole-file size is shown: {stdout}"
    );
    assert!(
        !stdout.contains(PROJECT_SENTINEL),
        "the project body must never appear: {stdout}"
    );
    assert!(
        !stdout.contains(PERSONAL_SENTINEL),
        "the personal body must never appear: {stdout}"
    );
    let _ = std::fs::remove_dir_all(&env.root);
}
