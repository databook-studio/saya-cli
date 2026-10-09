fn warm_up_saya() {
    // The first launch of a freshly linked binary can be slow on macOS (OS scan);
    // pay that one-time cost here, outside the tests' 15s timed windows.
    static WARM_UP: Once = Once::new();
    WARM_UP.call_once(|| {
        let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_saya"))
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("saya --version spawns");
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            if let Some(status) = child.try_wait().expect("saya --version is pollable") {
                assert!(status.success(), "saya --version exits successfully");
                return;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                panic!("saya did not start within 120s");
            }
            thread::sleep(Duration::from_millis(50));
        }
    });
}

fn run_pending_approval_cancellation(label: &str, elapsed_seconds: u64, input: Option<&[u8]>) {
    let root = scratch_root(label);
    let _cleanup = RootGuard(root.clone());
    let database = root.join("data.sqlite3");
    fs::write(&database, []).unwrap();
    let config_home = root.join("config-home");
    let saya_config = config_home.join("saya");
    fs::create_dir_all(&saya_config).unwrap();
    let config = saya_config.join("config.toml");
    fs::write(
        &config,
        format!(
            "[ai]\nmodel = 'mock'\nallow_data_sharing = true\n\
             investigation_max_elapsed_seconds = {elapsed_seconds}\n\n\
             [memory]\nmode = 'off'\n"
        ),
    )
    .unwrap();
    let connections = root.join("connections.toml");
    fs::write(
        &connections,
        format!(
            "[profiles.local]\ntype = 'sqlite'\npath = '{}'\n",
            database.display()
        ),
    )
    .unwrap();

    let mut provider = MockProvider::spawn();
    let pty = native_pty_system();
    let pair = pty
        .openpty(PtySize {
            rows: 24,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("native pty allocates");
    let mut command = CommandBuilder::new("sh");
    command.cwd(&root);
    command.args([
        "-c",
        "\"$@\"; status=$?; stty -a; IFS= read -r next; printf 'NEXT=%s\\n' \"$next\"; exit \"$status\"",
        "terminal-approval-pty",
        env!("CARGO_BIN_EXE_saya"),
        "--connections",
        connections.to_str().unwrap(),
        "--profile",
        "local",
        "--approval-mode",
        "ask",
        "--allow-data-sharing",
        "ask",
        "count rows",
    ]);
    command.env("HOME", &root);
    command.env("SAYA_CONFIG_HOME", &config_home);
    command.env("SAYA_STATE_DB", root.join("state.sqlite3"));
    command.env("SAYA_PROVIDER", "openai_compatible");
    command.env("SAYA_MODEL", "mock");
    command.env("SAYA_PROVIDER_BASE_URL", format!("{}/v1", provider.address));
    command.env("SAYA_API_KEY", "test-only");
    let mut child = pair.slave.spawn_command(command).expect("saya starts");
    let mut reader = pair.master.try_clone_reader().expect("pty reader");
    let mut writer = pair.master.take_writer().expect("pty writer");
    let (tx, rx) = std::sync::mpsc::channel();
    thread::spawn(move || {
        let mut bytes = [0_u8; 4096];
        while let Ok(count) = reader.read(&mut bytes) {
            if count == 0 || tx.send(bytes[..count].to_vec()).is_err() {
                break;
            }
        }
    });

    let started = Instant::now();
    let mut terminal = String::new();
    while !terminal.contains("[a] allow once") {
        assert!(
            started.elapsed() < DEADLINE,
            "approval never became pending; terminal:\n{terminal}"
        );
        if let Ok(bytes) = rx.recv_timeout(Duration::from_millis(100)) {
            terminal.push_str(&String::from_utf8_lossy(&bytes));
        }
    }
    assert_eq!(
        provider.requests(),
        1,
        "the real provider source started once"
    );
    if let Some(input) = input {
        writer
            .write_all(input)
            .expect("bare Esc reaches the terminal owner");
    }

    while !terminal.contains("icanon") {
        assert!(
            started.elapsed() < DEADLINE,
            "deadline did not restore terminal ownership; terminal:\n{terminal}"
        );
        if let Ok(bytes) = rx.recv_timeout(Duration::from_millis(100)) {
            terminal.push_str(&String::from_utf8_lossy(&bytes));
        }
    }
    writer
        .write_all(b"next question\r")
        .expect("the next tty owner accepts its line");
    while child.try_wait().expect("child is pollable").is_none() {
        assert!(
            started.elapsed() < DEADLINE,
            "terminal wrapper did not settle"
        );
        if let Ok(bytes) = rx.recv_timeout(Duration::from_millis(100)) {
            terminal.push_str(&String::from_utf8_lossy(&bytes));
        }
    }
    let status = child.try_wait().unwrap().expect("child settled");
    // The typed stop classes: a user-cancelled pending approval latches the
    // question token with User and exits the cancelled class (130); a question
    // stopped by its configured elapsed budget latches Budget and exits the
    // partial/paused class (6). The ask adapter classifies from the token's
    // first stop reason.
    let expected_exit = if input.is_some() { 130 } else { 6 };
    assert_eq!(
        status.exit_code(),
        expected_exit,
        "cancelled question has its truthful stop class: {terminal}"
    );
    if input.is_some() {
        assert!(
            terminal.contains("request cancelled"),
            "bare Esc must reach the shared cancellation path: {terminal}"
        );
    } else {
        assert!(
            terminal.contains("elapsed-time budget"),
            "the budget-stopped question names its configured budget: {terminal}"
        );
    }
    assert_eq!(
        provider.requests(),
        1,
        "cancellation while approval waited must not start another provider/tool round"
    );
    assert!(
        !terminal.contains("Tool completed"),
        "an unapproved, cancelled call cannot be promoted as successful evidence: {terminal}"
    );
    while let Ok(bytes) = rx.recv_timeout(Duration::from_secs(1)) {
        terminal.push_str(&String::from_utf8_lossy(&bytes));
        if terminal.contains("icanon") {
            break;
        }
    }
    assert!(
        terminal.contains("icanon") && !terminal.contains("-icanon"),
        "the approval guard must restore canonical terminal mode: {terminal}"
    );
    assert!(
        terminal.contains("NEXT=next question"),
        "the next tty owner, not the cancelled approval, must receive its line: {terminal}"
    );
    provider.join();
}
