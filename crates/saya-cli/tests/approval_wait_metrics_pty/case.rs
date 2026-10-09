/// The shared section assertions: the header printed, the expected
/// approval_wait line, and none of the prompt/SQL/answer sentinels inside
/// the fixed section.
fn assert_metrics_section(terminal: &str, expected: &str) {
    assert!(
        terminal.contains("saya local metrics"),
        "the metrics section prints for a verbose run: {terminal}"
    );
    assert!(
        terminal.contains(expected),
        "expected the approval_wait line `{expected}`: {terminal}"
    );
    let start = terminal.find("saya local metrics").expect("header present");
    let section = &terminal[start..];
    assert!(
        !section.contains("allow once")
            && !section.contains("SELECT 1")
            && !section.contains("done"),
        "the fixed metrics section carries no prompt, SQL, or answer text: {section}"
    );
}

fn warm_up_saya() {
    // The first launch of a freshly linked binary can be slow on macOS (OS
    // scan); pay that one-time cost here, outside the tests' 15s windows.
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

/// One verbose ask under a real PTY: the mock provider's first response
/// requests the SQL tool; in `ask` mode the harness answers the approval
/// prompt with `a` + CR once raw mode owns the terminal, in every mode the
/// run must settle green with the tool run and its follow-up round, and the
/// printed metrics section returned for the caller's assertions.
fn run_approval_metrics_case(label: &str, approval_mode: &str, answer: Option<&[u8]>) -> String {
    let root = scratch_root(label);
    let _cleanup = RootGuard(root.clone());
    let database = root.join("data.sqlite3");
    fs::write(&database, []).unwrap();
    let config_home = root.join("config-home");
    let saya_config = config_home.join("saya");
    fs::create_dir_all(&saya_config).unwrap();
    fs::write(
        saya_config.join("config.toml"),
        "[ai]\nmodel = 'mock'\nallow_data_sharing = true\n\
         investigation_max_elapsed_seconds = 60\n\n[memory]\nmode = 'off'\n",
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
    let mut provider = MockProvider::spawn_with_follow_up();
    let pty = native_pty_system();
    let pair = pty
        .openpty(PtySize {
            rows: 24,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("native pty allocates");
    let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_saya"));
    command.cwd(&root);
    command.args([
        "--verbose",
        "--connections",
        connections.to_str().unwrap(),
        "--profile",
        "local",
        "--approval-mode",
        approval_mode,
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
    let raw_fd = pair
        .master
        .as_raw_fd()
        .expect("native PTY exposes a file descriptor");
    let mut reader = pair.master.try_clone_reader().expect("pty reader");
    let mut writer = pair.master.take_writer().expect("pty writer");
    let (tx, rx) = std::sync::mpsc::channel();
    let reader_thread = PtyReader::new(thread::spawn(move || {
        let mut bytes = [0_u8; 4096];
        while let Ok(count) = reader.read(&mut bytes) {
            if count == 0 || tx.send(bytes[..count].to_vec()).is_err() {
                break;
            }
        }
    }));
    let mut child = OwnedChild::new(pair.slave.spawn_command(command).expect("saya starts"));
    let started = Instant::now();
    let mut terminal = String::new();
    if let Some(answer) = answer {
        while !terminal.contains("[a] allow once") {
            assert!(
                started.elapsed() < DEADLINE,
                "approval did not become pending: {terminal}"
            );
            if let Ok(bytes) = rx.recv_timeout(Duration::from_millis(100)) {
                terminal.push_str(&String::from_utf8_lossy(&bytes));
            }
        }
        loop {
            let mut termios = unsafe { std::mem::zeroed::<libc::termios>() };
            assert_eq!(
                unsafe { libc::tcgetattr(raw_fd, &mut termios) },
                0,
                "termios reads"
            );
            if termios.c_lflag & libc::ICANON == 0 {
                break;
            }
            assert!(
                started.elapsed() < DEADLINE,
                "the approval prompt printed without its raw terminal owner: {terminal}"
            );
            thread::sleep(Duration::from_millis(10));
        }
        writer
            .write_all(answer)
            .expect("the answer key reaches the raw terminal owner");
    }
    while !terminal.contains("saya local metrics") {
        assert!(
            started.elapsed() < DEADLINE,
            "the metrics section never printed: {terminal}"
        );
        if let Ok(bytes) = rx.recv_timeout(Duration::from_millis(100)) {
            terminal.push_str(&String::from_utf8_lossy(&bytes));
        }
    }
    while child.try_wait().expect("child is pollable").is_none() {
        assert!(
            started.elapsed() < DEADLINE,
            "the metrics run did not settle: {terminal}"
        );
        if let Ok(bytes) = rx.recv_timeout(Duration::from_millis(100)) {
            terminal.push_str(&String::from_utf8_lossy(&bytes));
        }
    }
    assert_eq!(
        child.try_wait().unwrap().unwrap().exit_code(),
        0,
        "the metrics run succeeds: {terminal}"
    );
    child.disarm();
    assert_eq!(
        provider.requests(),
        2,
        "the tool result returns to the provider exactly once: {terminal}"
    );
    while let Ok(bytes) = rx.recv_timeout(Duration::from_millis(300)) {
        terminal.push_str(&String::from_utf8_lossy(&bytes));
    }
    drop(writer);
    drop(pair);
    reader_thread.join();
    provider.join();
    terminal
}
