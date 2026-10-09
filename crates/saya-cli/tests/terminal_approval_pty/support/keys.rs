fn run_approval_key(label: &str, input: &[u8], allowed: bool) {
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
        .write_all(input)
        .expect("test key reaches raw terminal owner");
    while child.try_wait().expect("child is pollable").is_none() {
        assert!(
            started.elapsed() < DEADLINE,
            "approval child did not settle: {terminal}"
        );
        if let Ok(bytes) = rx.recv_timeout(Duration::from_millis(100)) {
            terminal.push_str(&String::from_utf8_lossy(&bytes));
        }
    }
    assert_eq!(
        child.try_wait().unwrap().unwrap().exit_code(),
        0,
        "approval run succeeds"
    );
    child.disarm();
    let follow_up = provider.join_follow_up();
    assert_eq!(
        provider.requests(),
        2,
        "tool result returns to the provider exactly once"
    );
    let paired = paired_tool_result(&follow_up);
    if allowed {
        let result: saya_types::QueryResult =
            serde_json::from_str(&paired).expect("plain a returns a structured SQL result");
        assert_eq!(result.rows, vec![serde_json::json!([1])]);
        assert_eq!(result.row_count, 1);
        assert!(!result.truncated);
        assert!(result.executed_sql.contains("SELECT 1"));
    } else {
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&paired)
                .expect("modified input returns a structured denial")["error"],
            "tool call denied by approval policy",
            "modified input denies the paired tool call"
        );
    }
    drop(writer);
    drop(pair);
    reader_thread.join();
}
