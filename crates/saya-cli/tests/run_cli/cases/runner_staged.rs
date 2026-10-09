/// A correctly staged program reaches the run: the probe proves the host,
/// the admission check approves, and a plan whose step asks for the runner
/// scope — the thing `--allow runner:bench` granted — binds, is approved
/// headlessly, and completes. (macOS: the platform the probe proves.)
#[cfg(target_os = "macos")]
#[test]
fn a_correctly_staged_program_reaches_a_runner_asking_step() {
    let env = test_root("runner-happy");
    let programs = env.root.join("staged-programs");
    fs::create_dir_all(&programs).unwrap();
    fs::copy("/bin/echo", programs.join("bench")).unwrap();
    user_config(
        &env,
        &format!(
            "[jobs.runner]\nallow = [\"bench\"]\nprogram_dir = {}\n",
            toml_path(&programs)
        ),
    );
    let (address, _ready) = mock(vec![
        Scripted {
            body: plan_body_with_capabilities(&[(
                "run the benchmark harness",
                serde_json::json!({"runner": {"programs": ["bench"]}}),
            )]),
            delay_ms: 0,
        },
        Scripted {
            body: sse("measured"),
            delay_ms: 0,
        },
    ]);
    let started = saya(
        &env,
        &[
            "--non-interactive",
            "run",
            "--allow",
            "runner:bench",
            "measure the harness",
        ],
        &address,
    );
    assert_eq!(
        started.status.code(),
        Some(0),
        "a run with a correctly staged program must start and complete; stderr: {}",
        stderr(&started)
    );
    let journal = fs::read_to_string(
        fs::read_dir(&env.runs)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path()
            .join("events.ndjson"),
    )
    .unwrap();
    assert!(
        journal.contains("run_started") && journal.contains("\"completed\""),
        "the runner-approved run started and completed: {journal}"
    );
    let _ = fs::remove_dir_all(&env.root);
}
