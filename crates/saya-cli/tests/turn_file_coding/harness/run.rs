/// The stderr of one blocking run: the diagnostics a scripted run said.
pub fn stderr(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The shared headless invocation: `saya --workspace <project> --turn-file
/// <turn> --approval-mode <mode> --allow-data-sharing --format ndjson` plus
/// `extra` arguments (`--allow` seeds, `--resume`), with NO connections file
/// and NO profile, isolated config/state/session dirs, and the scripted
/// provider behind `SAYA_PROVIDER_BASE_URL`. stdin is null throughout: the
/// run never prompts.
fn run_with_extra(
    env: &Env,
    scripted: &Scripted,
    mode: &str,
    turn: &str,
    extra: &[&str],
) -> std::process::Output {
    let turn_file = env.root.join("turn.txt");
    std::fs::write(&turn_file, turn).unwrap();
    let mut args = vec![
        "--workspace",
        env.project.to_str().unwrap(),
        "--turn-file",
        turn_file.to_str().unwrap(),
        "--approval-mode",
        mode,
        "--allow-data-sharing",
        "--format",
        "ndjson",
    ];
    args.extend_from_slice(extra);
    Command::new(env!("CARGO_BIN_EXE_saya"))
        .args(args)
        .current_dir(&env.root)
        .env("SAYA_CONFIG_HOME", &env.root)
        .env("SAYA_SESSION_DIR", &env.sessions)
        .env("SAYA_STATE_DB", env.root.join("state.sqlite3"))
        .env("SAYA_PROVIDER", "openai_compatible")
        .env("SAYA_MODEL", "mock-model")
        .env("SAYA_PROVIDER_BASE_URL", format!("{}/v1", scripted.address))
        .env("SAYA_API_KEY", "mock-secret")
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

/// `run_with_extra` with no launch grant: the unseeded run.
pub fn run(env: &Env, scripted: &Scripted, mode: &str, turn: &str) -> std::process::Output {
    run_with_extra(env, scripted, mode, turn, &[])
}

/// `run` plus one `--allow <token>` per entry: the launch statement's grant
/// seeds, landed in the grant store on a fresh start (`seed_launch_allow`).
pub fn run_allowing(
    env: &Env,
    scripted: &Scripted,
    mode: &str,
    turn: &str,
    allow: &[&str],
) -> std::process::Output {
    let mut extra = Vec::new();
    for token in allow {
        extra.push("--allow");
        extra.push(token);
    }
    run_with_extra(env, scripted, mode, turn, &extra)
}

/// A resumed run: `--resume <id>` re-opens the saved record from the same
/// session dir. `allow` seeds are empty for the contract under test —
/// grants die with the process — and carried only by the mutation that
/// proves the test bites.
pub fn run_resuming(
    env: &Env,
    scripted: &Scripted,
    mode: &str,
    turn: &str,
    session_id: &str,
    allow: &[&str],
) -> std::process::Output {
    let mut extra = vec!["--resume", session_id];
    for token in allow {
        extra.push("--allow");
        extra.push(token);
    }
    run_with_extra(env, scripted, mode, turn, &extra)
}
