/// A nested `saya run` started from a session `/run` passes its whole stream
/// through the parent's stdout unmangled: the parent never re-tags a child
/// line into its own envelope and never swallows one. The exact child line
/// (`{"type":"run_started"}`) appears in the parent's stdout byte-for-byte.
#[test]
fn a_nested_run_child_lines_survive_to_the_parent_stdout_unmangled() {
    let env = test_root("nested-child");
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
    // The parent session, piped: one `/run` line in NDJSON mode. The child
    // inherits the parent's env, so the run lands in the same scratch tree.
    let mut parent = ProcessCommand::new(env!("CARGO_BIN_EXE_saya"))
        .args(["--format", "ndjson"])
        .current_dir(&env.root)
        .env("SAYA_CONFIG_HOME", env.root.join("user-config"))
        .env("SAYA_RUNS_DIR", &env.runs)
        .env("SAYA_STATE_DB", &env.state)
        .env("SAYA_PROVIDER", "openai_compatible")
        .env("SAYA_MODEL", "mock-model")
        .env("SAYA_PROVIDER_BASE_URL", format!("{address}/v1"))
        .env("SAYA_API_KEY", "mock-secret")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    parent
        .stdin
        .take()
        .unwrap()
        .write_all(b"/run survey the data --allow workspace-write\n")
        .unwrap();
    let output = parent.wait_with_output().unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "the session survives the nested run; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let parent_stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    // The child's own first line, byte for byte — not wrapped, re-tagged, or
    // dropped: the parent re-emits nothing, it passed the file descriptors
    // through.
    let run_started = format!("{}\n", serde_json::json!({"type": "run_started"}));
    assert!(
        parent_stdout.contains(&run_started),
        "the child's run_started line must survive unmangled: {parent_stdout:?}"
    );
    // And every line the parent emitted is still one JSON object per line —
    // the child's stream is a valid stream inside the parent's.
    let values: Vec<serde_json::Value> = parent_stdout
        .lines()
        .map(|line| {
            serde_json::from_str(line)
                .unwrap_or_else(|error| panic!("parent line not JSON: {line:?}: {error}"))
        })
        .collect();
    assert!(
        values
            .iter()
            .any(|value| value.get("type") == Some(&serde_json::json!("completed"))),
        "the child's run completed on the parent's stream: {values:?}"
    );

    // The run the child made is the one the session's /runs can see: same
    // runs root, same store.
    let listing = saya(&env, &["run", "list"], &address);
    assert!(
        stdout(&listing).contains("completed"),
        "the nested run is visible to `saya run list`: {}",
        stdout(&listing)
    );
    let _ = fs::remove_dir_all(&env.root);
}
// ---------------------------------------------------------------------------
// Interactive approval on a real terminal: the binary's stdin is a PTY, so
// `can_prompt` holds and the bound plan is approved through the channel —
// the `tui/agent.rs` pattern with the terminal answering. The approval view
// is the one interaction; the run then proceeds without asking again.
// ---------------------------------------------------------------------------
