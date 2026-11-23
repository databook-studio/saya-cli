// The launch: a scratch HOME's config and one `demo` profile, a pty at the
// caller's size, and the real binary — the one `SAYA_PTY_BINARY` names, or the
// one this test binary was built beside.

fn scratch_home(tag: &str) -> PathBuf {
    let home = std::env::temp_dir().join(format!("saya-ui7-capture-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).expect("the scratch home builds");
    home
}

fn launch(home: &Path, base_url: &str, (cols, rows): (u16, u16)) -> Cap {
    let config = home.join("config.toml");
    std::fs::write(
        &config,
        format!("[ai]\nmodel = '{MODEL}'\nallow_data_sharing = true\n\n[run]\nmax_rows = 10\n"),
    )
    .expect("the config writes");
    let database = home.join("demo.sqlite3");
    std::fs::write(&database, b"").expect("the demo database writes");
    let connections = home.join("connections.toml");
    std::fs::write(
        &connections,
        format!(
            "[profiles.demo]\ntype = 'sqlite'\npath = '{}'\nread_only = false\n",
            database.display()
        ),
    )
    .expect("the connections write");
    let launch_dir = home.join("launch-dir");
    std::fs::create_dir_all(&launch_dir).expect("the launch dir builds");

    let pair = native_pty_system()
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("a pty opens");
    let binary = std::env::var_os("SAYA_PTY_BINARY")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_saya")));
    let mut command = CommandBuilder::new(binary);
    for (key, value) in [
        ("HOME", home.to_path_buf()),
        ("SAYA_CONFIG_HOME", home.join("config-home")),
        ("SAYA_SESSION_DIR", home.join("sessions")),
        ("SAYA_STATE_DB", home.join("state.sqlite3")),
        ("TMPDIR", launch_dir.clone()),
        ("TMP", launch_dir.clone()),
        ("TEMP", launch_dir.clone()),
        ("SAYA_PROVIDER", PathBuf::from("openai_compatible")),
        ("SAYA_MODEL", PathBuf::from(MODEL)),
        ("SAYA_PROVIDER_BASE_URL", PathBuf::from(base_url)),
        ("SAYA_API_KEY", PathBuf::from("test-only")),
    ] {
        command.env(key, value);
    }
    command.env_remove("XDG_CONFIG_HOME");
    command.env_remove("XDG_DATA_HOME");
    command.env_remove("APPDATA");
    command.cwd(&launch_dir);
    command.args([
        "--config",
        config.to_str().expect("utf-8 config path"),
        "--connections",
        connections.to_str().expect("utf-8 connections path"),
        "--approval-mode",
        "read-only",
    ]);
    let child = pair.slave.spawn_command(command).expect("saya spawns");
    drop(pair.slave);
    let writer = pair.master.take_writer().expect("a pty writer");
    let mut reader = pair.master.try_clone_reader().expect("a pty reader");
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let mut buf = vec![0_u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => {
                    let _ = tx.send(Ok(Vec::new()));
                    break;
                }
                Ok(count) => {
                    if tx.send(Ok(buf[..count].to_vec())).is_err() {
                        break;
                    }
                }
                Err(error) => {
                    let _ = tx.send(Err(error));
                    break;
                }
            }
        }
    });
    Cap {
        writer,
        parser: vt100::Parser::new(rows, cols, 0),
        rx,
        child,
    }
}
