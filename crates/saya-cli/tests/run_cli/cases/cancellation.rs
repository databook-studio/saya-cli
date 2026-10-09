/// Ctrl-C cancels a run: exit 130, the documented cancelled class.
#[cfg(unix)]
#[test]
fn ctrl_c_cancels_a_run_with_exit_130() {
    let env = test_root("ctrl-c");
    // Keep the plan response held after SIGINT so the command's local source
    // owner can be observed before it is explicitly released.
    let (address, ready, release) = held_mock(plan_body(&["never reached"]));
    let mut child = ProcessCommand::new(env!("CARGO_BIN_EXE_saya"))
        .args([
            "--non-interactive",
            "run",
            "--allow",
            "workspace-write",
            "a long goal",
        ])
        .current_dir(&env.root)
        .env("SAYA_CONFIG_HOME", env.root.join("user-config"))
        .env("SAYA_RUNS_DIR", &env.runs)
        .env("SAYA_STATE_DB", &env.state)
        .env("SAYA_PROVIDER", "openai_compatible")
        .env("SAYA_MODEL", "mock-model")
        .env("SAYA_PROVIDER_BASE_URL", format!("{address}/v1"))
        .env("SAYA_API_KEY", "mock-secret")
        .spawn()
        .unwrap();
    // Wait until the child is provably inside its provider call — the
    // Ctrl-C handler is armed by then, because the select! that drives the
    // run registered it before the plan proposal started.
    wait_ready(&ready);
    // SAFETY: `kill` takes no pointers and has no invariants to uphold.
    let sent = unsafe { libc::kill(child.id() as libc::pid_t, libc::SIGINT) };
    assert_eq!(sent, 0, "the child must be alive to receive SIGINT");
    for _ in 0..20 {
        assert!(
            child.try_wait().unwrap().is_none(),
            "Ctrl-C must retain the active drive until the source response settles"
        );
        thread::sleep(Duration::from_millis(10));
    }
    release.send(()).unwrap();
    let status = child.wait().unwrap();
    assert_eq!(
        status.code(),
        Some(130),
        "Ctrl-C must exit 130 through the handler, not die by signal"
    );
    let _ = fs::remove_dir_all(&env.root);
}

/// Resume Ctrl-C settles its active journal owner before the process exits.
/// The provider stream is locally abandoned on stop, so this proves local
/// journal/lock settlement only; it makes no claim about remote completion.
#[cfg(unix)]
#[test]
fn resume_ctrl_c_records_cancelled_before_releasing_the_run_lock() {
    let env = test_root("resume-ctrl-c");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let planner_seen = Arc::new(AtomicBool::new(false));
    let step_seen = Arc::new(AtomicBool::new(false));
    let planner_flag = Arc::clone(&planner_seen);
    let step_flag = Arc::clone(&step_seen);
    let (release_initial, initial_wait) = mpsc::channel();
    thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut request = [0_u8; 32_768];
            let _ = stream.read(&mut request);
            planner_flag.store(true, Ordering::SeqCst);
            let body = plan_body(&["interrupted step"]);
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.flush();
        }
        if let Ok((mut stream, _)) = listener.accept() {
            let mut request = [0_u8; 32_768];
            let _ = stream.read(&mut request);
            step_flag.store(true, Ordering::SeqCst);
            let _ = initial_wait.recv();
            let body = sse("done");
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.flush();
        }
    });
    let mut interrupted = ChildGuard(
        ProcessCommand::new(env!("CARGO_BIN_EXE_saya"))
            .args(["--non-interactive", "run", "--allow", "none", "a long goal"])
            .current_dir(&env.root)
            .env("SAYA_CONFIG_HOME", env.root.join("user-config"))
            .env("SAYA_RUNS_DIR", &env.runs)
            .env("SAYA_STATE_DB", &env.state)
            .env("SAYA_PROVIDER", "openai_compatible")
            .env("SAYA_MODEL", "mock-model")
            .env("SAYA_PROVIDER_BASE_URL", format!("{address}/v1"))
            .env("SAYA_API_KEY", "mock-secret")
            .spawn()
            .unwrap(),
    );
    wait_ready(&planner_seen);
    wait_ready(&step_seen);
    interrupted.kill().unwrap();
    let _ = interrupted.wait().unwrap();
    release_initial.send(()).unwrap();

    let listing = saya(&env, &["run", "list"], &address);
    assert_eq!(listing.status.code(), Some(0), "{}", stderr(&listing));
    let id = newest_run_id(&listing);
    let (resume_address, ready, release) = held_mock(sse("done"));
    let mut resumed = ChildGuard(
        ProcessCommand::new(env!("CARGO_BIN_EXE_saya"))
            .args([
                "--non-interactive",
                "run",
                "resume",
                &id,
                "--retry-incomplete",
            ])
            .current_dir(&env.root)
            .env("SAYA_CONFIG_HOME", env.root.join("user-config"))
            .env("SAYA_RUNS_DIR", &env.runs)
            .env("SAYA_STATE_DB", &env.state)
            .env("SAYA_PROVIDER", "openai_compatible")
            .env("SAYA_MODEL", "mock-model")
            .env("SAYA_PROVIDER_BASE_URL", format!("{resume_address}/v1"))
            .env("SAYA_API_KEY", "mock-secret")
            .spawn()
            .unwrap(),
    );
    wait_ready(&ready);
    // SAFETY: `kill` takes no pointers and has no invariants to uphold.
    let sent = unsafe { libc::kill(resumed.id() as libc::pid_t, libc::SIGINT) };
    assert_eq!(sent, 0, "the resumed process is inside its provider call");
    let mut status = None;
    for _ in 0..200 {
        if let Some(exited) = resumed.try_wait().unwrap() {
            status = Some(exited);
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    let settled_before_remote_release = status.is_some();
    if !settled_before_remote_release {
        let _ = resumed.kill();
        let _ = resumed.wait();
    }
    release.send(()).unwrap();
    assert!(
        settled_before_remote_release,
        "the local provider attempt must settle without claiming remote completion"
    );
    assert_eq!(status.unwrap().code(), Some(130));
    let journal = fs::read_to_string(env.runs.join(&id).join("events.ndjson")).unwrap();
    assert_eq!(
        journal
            .lines()
            .filter(|line| line.contains("\"type\":\"cancelled\""))
            .count(),
        1,
        "resume records exactly one durable cancellation before releasing its lock"
    );
    let _ = fs::remove_dir_all(&env.root);
}
