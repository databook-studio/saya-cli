/// What one interactive run leaves behind: the exit code and the captured
/// terminal stream, for assertions on both the prompt and the completion.
struct InteractiveRun {
    exit_code: Option<i32>,
    stream: String,
}

/// Answers terminal cursor-position queries so the PTY can render the prompt
/// on platforms where the child waits for a Device Status Report.
fn answer_cursor_queries(stream: &str, answered: &mut usize, writer: &mut dyn Write) {
    let queries = stream.matches("\x1b[6n").count();
    while *answered < queries {
        writer
            .write_all(b"\x1b[1;1R")
            .expect("the pty cursor-position response");
        *answered += 1;
    }
}

/// Spawns `saya run` on a PTY, waits for the approval prompt to appear,
/// answers with `answer`, and waits for the child to exit. A timeout kills
/// the child and fails the test — an interactive ask that never resolves is
/// a hang, not a pass.
fn run_interactively(env: &TestEnv, args: &[&str], address: &str, answer: &str) -> InteractiveRun {
    use portable_pty::{CommandBuilder, PtySize, native_pty_system};
    const DEADLINE: Duration = Duration::from_secs(30);
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: 24,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("the pty must be allocatable");
    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_saya"));
    // The headless helper runs the child in the scratch root; on a PTY the
    // builder would otherwise fall back to $HOME as the cwd, where a real
    // project-layer `.saya/config.toml` would shadow the scratch tree.
    cmd.cwd(&env.root);
    cmd.args(args);
    cmd.env("SAYA_CONFIG_HOME", env.root.join("user-config"));
    cmd.env("SAYA_RUNS_DIR", &env.runs);
    cmd.env("SAYA_STATE_DB", &env.state);
    cmd.env("SAYA_PROVIDER", "openai_compatible");
    cmd.env("SAYA_MODEL", "mock-model");
    cmd.env("SAYA_PROVIDER_BASE_URL", format!("{address}/v1"));
    cmd.env("SAYA_API_KEY", "mock-secret");
    let mut child = pair.slave.spawn_command(cmd).expect("the child must spawn");
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().expect("the pty reader");
    let mut writer = pair.master.take_writer().expect("the pty writer");

    // Drain the terminal stream on a thread; the main loop watches for the
    // prompt and then for the exit.
    let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
    thread::spawn(move || {
        let mut buffer = [0_u8; 4096];
        while let Ok(n) = reader.read(&mut buffer) {
            if n == 0 || tx.send(buffer[..n].to_vec()).is_err() {
                break;
            }
        }
    });

    let mut stream = String::new();
    let mut cursor_queries_answered = 0;
    let started = std::time::Instant::now();
    let prompt_seen = loop {
        if started.elapsed() > DEADLINE {
            let _ = child.kill();
            panic!("the approval prompt never appeared; stream:\n{stream}");
        }
        if let Ok(bytes) = rx.recv_timeout(Duration::from_millis(100)) {
            stream.push_str(&String::from_utf8_lossy(&bytes));
            answer_cursor_queries(&stream, &mut cursor_queries_answered, &mut writer);
        }
        if stream.contains("Approve this plan") {
            break true;
        }
    };
    let _ = prompt_seen;
    // The one answer: an explicit yes — nothing else approves.
    // ConPTY treats carriage return as the Enter key. Sending LF is echoed
    // as text but does not complete stdin's read_line on Windows.
    #[cfg(windows)]
    let answer = answer.replace('\n', "\r");
    writer.write_all(answer.as_bytes()).expect("the pty writer");
    while child
        .try_wait()
        .expect("the child must be pollable")
        .is_none()
    {
        if started.elapsed() > DEADLINE {
            let _ = child.kill();
            panic!("the run never exited after the approval; stream:\n{stream}");
        }
        if let Ok(bytes) = rx.recv_timeout(Duration::from_millis(50)) {
            stream.push_str(&String::from_utf8_lossy(&bytes));
            answer_cursor_queries(&stream, &mut cursor_queries_answered, &mut writer);
        }
    }
    let exit_code = child
        .try_wait()
        .expect("the child exited")
        .map(|status| status.exit_code() as i32);
    // Drain whatever remains (the run's stdout) for the completion asserts.
    while let Ok(bytes) = rx.try_recv() {
        stream.push_str(&String::from_utf8_lossy(&bytes));
    }
    InteractiveRun { exit_code, stream }
}

/// The journal of the one run in the scratch tree.
fn only_run_journal(env: &TestEnv) -> String {
    let mut entries = fs::read_dir(&env.runs)
        .expect("the runs root exists")
        .flatten()
        .map(|entry| entry.path())
        .collect::<Vec<PathBuf>>();
    assert_eq!(entries.len(), 1, "exactly one run exists: {entries:?}");
    fs::read_to_string(entries.remove(0).join("events.ndjson")).expect("the journal is readable")
}
