/// The submitted parameter value. Synthetic — shaped like the audit's
/// reproduction value (not a credential pattern), so only the param
/// redaction can keep it off disk; the generic redaction alone never would.
const PARAM_VALUE: &str = "synthetic-confidential-customer-922";
/// The redacted spelling every persisted/echoed copy must show instead.
const PARAM_REDACTED: &str = "--param label=…";

/// Seeds the demo database with one matching row (id 4242) and a decoy, so
/// the run's result is non-empty only when the submitted value actually
/// reached the query: the run refuses (missing required parameter) if the
/// value never arrives, and the decoy row keeps a wrong/empty binding empty.
fn seed_sentinel(database: &Path, value: &str) {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test tokio runtime builds")
        .block_on(async {
            let options = sqlx::sqlite::SqliteConnectOptions::new()
                .filename(database)
                .create_if_missing(true);
            let pool = sqlx::SqlitePool::connect_with(options)
                .await
                .expect("the demo db opens for seeding");
            sqlx::query("CREATE TABLE sentinel (id INTEGER PRIMARY KEY, label TEXT NOT NULL)")
                .execute(&pool)
                .await
                .expect("the sentinel table is created");
            sqlx::query("INSERT INTO sentinel (id, label) VALUES (1, 'unrelated-row'), (4242, ?1)")
                .bind(value)
                .execute(&pool)
                .await
                .expect("the sentinel rows are inserted");
            pool.close().await;
        });
}

/// Saves the parameterized investigation headlessly under the same isolated
/// env the TUI child gets (`tui_command`'s variables): the document lands
/// beside the state DB — the investigations root the TUI's composed runtime
/// reads too. Returns the document id (the saved file's stem).
fn save_param_investigation(home: &Path) -> String {
    let output = std::process::Command::new(saya_bin())
        .env("HOME", home)
        .env("SAYA_CONFIG_HOME", home.join("config-home"))
        .env("SAYA_SESSION_DIR", home.join("sessions"))
        .env("SAYA_STATE_DB", home.join("state.sqlite3"))
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_DATA_HOME")
        .env_remove("APPDATA")
        .env_remove("SAYA_API_KEY")
        .env_remove("SAYA_AI_API_KEY")
        .args([
            "--non-interactive",
            "--format",
            "json",
            "--config",
            home.join("config.toml").to_str().unwrap(),
            "--connections",
            home.join("connections.toml").to_str().unwrap(),
            "investigation",
            "save",
            "--name",
            "By sentinel label",
            "--sql",
            "SELECT id AS match_id FROM sentinel WHERE label = :label",
            "--param-spec",
            "label:string:required",
        ])
        .output()
        .expect("the headless investigation save spawns");
    assert_eq!(
        output.status.code(),
        Some(0),
        "the investigation save succeeded: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let root = home.join("investigations");
    let mut ids: Vec<String> = std::fs::read_dir(&root)
        .expect("the investigations root exists")
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
        .map(|entry| {
            entry
                .path()
                .file_stem()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    ids.sort();
    assert_eq!(
        ids.len(),
        1,
        "exactly one investigation was saved under {root:?}: {ids:?}"
    );
    ids.remove(0)
}

/// Writes the config + connections the param-redaction case shares between
/// the headless save and the TUI child: the offline smoke config pinning
/// `default_profile = 'demo'` (what the save and the run both bind against)
/// and one writable sqlite `demo` profile over the seeded database.
fn write_param_redact_config(home: &Path) {
    std::fs::write(
        home.join("config.toml"),
        "default_profile = 'demo'\n\n[ai]\nmodel = 'smoke'\n\n[run]\nmax_rows = 10\n",
    )
    .expect("param-redact config writes");
    std::fs::write(
        home.join("connections.toml"),
        format!(
            "[profiles.demo]\ntype = 'sqlite'\npath = '{}'\nread_only = false\n",
            home.join("demo.sqlite3").display()
        ),
    )
    .expect("param-redact connections write");
}

/// Spawns the TUI for the param-redaction case: the config the headless save
/// already used, and the seeded database file left untouched (never
/// truncated), so the sentinel row survives into the TUI's session.
fn param_redact_tui(home: &Path) -> Result<InteractiveTui, String> {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: ROWS,
            cols: COLS,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("could not allocate pty: {e}"))?;
    // History must be on for the persistence assertions to mean anything,
    // whatever the outer shell sets.
    let mut cmd = tui_command(
        home,
        &home.join("config.toml"),
        &home.join("connections.toml"),
    );
    cmd.env_remove("SAYA_HISTORY");
    let child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| format!("could not spawn saya on pty: {e}"))?;
    drop(pair.slave);
    let writer = pair
        .master
        .take_writer()
        .map_err(|e| format!("could not take pty writer: {e}"))?;
    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| format!("could not clone pty reader: {e}"))?;
    let rx = reader_channel(reader);
    Ok(InteractiveTui {
        writer,
        parser: vt100::Parser::new(ROWS, COLS, 0),
        rx,
        child: ChildGuard::new(child),
        raw: Vec::new(),
        cursor_replies: 0,
    })
}

/// Boundedly waits until at least one saved-session JSON exists under the
/// session dir (the dispatch path saves after every turn). A timeout prints
/// a note rather than failing — the absence assertions then run over
/// whatever exists, which is the binding check.
fn wait_for_session_file(sessions: &Path, within: Duration) {
    let start = Instant::now();
    while start.elapsed() < within {
        if std::fs::read_dir(sessions)
            .map(|entries| entries.count() > 0)
            .unwrap_or(false)
        {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    eprintln!("note: no session file was written under {:?}", sessions);
}

/// Every file under `root`, recursively (the session store's layout).
fn files_under(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                files.push(path);
            }
        }
    }
    files
}
