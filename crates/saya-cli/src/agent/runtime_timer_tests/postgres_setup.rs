const fn finite(value: u64) -> BudgetLimit {
    BudgetLimit::Finite(NonZeroU64::new(value).expect("test budget is nonzero"))
}

fn temp_root() -> std::path::PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "saya-runtime-answering-timer-{}-{stamp}",
        std::process::id()
    ));
    std::fs::create_dir_all(&root).expect("create test root");
    root
}

fn postgres_setup_source(
    setup_started: tokio::sync::oneshot::Sender<()>,
    release: mpsc::Receiver<()>,
) -> (u16, thread::JoinHandle<Result<(), String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind PostgreSQL setup source");
    let port = listener
        .local_addr()
        .expect("PostgreSQL source address")
        .port();
    let source = thread::spawn(move || {
        listener
            .set_nonblocking(true)
            .map_err(|error| error.to_string())?;
        let deadline = std::time::Instant::now() + Duration::from_secs(4);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if release.try_recv().is_ok() {
                        return Err("setup source released before startup".into());
                    }
                    if std::time::Instant::now() >= deadline {
                        return Err("setup source timed out waiting for startup".into());
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                Err(error) => return Err(error.to_string()),
            }
        };
        // Accepted sockets inherit O_NONBLOCK from a non-blocking listener on macOS.
        stream
            .set_nonblocking(false)
            .map_err(|error| error.to_string())?;
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .map_err(|error| error.to_string())?;
        let mut startup = [0_u8; 32];
        let bytes = stream
            .read(&mut startup)
            .map_err(|error| error.to_string())?;
        if bytes == 0 {
            return Err("PostgreSQL client closed before startup bytes".into());
        }
        setup_started
            .send(())
            .map_err(|_| "runtime stopped before setup observation".to_string())?;
        release
            .recv_timeout(Duration::from_secs(4))
            .map_err(|_| "test did not release PostgreSQL setup".to_string())?;
        Ok(())
    });
    (port, source)
}

async fn runtime_for_postgres_setup(root: &std::path::Path, port: u16) -> RuntimeConfig {
    let profile = DatabaseProfile::Postgres {
        host: "127.0.0.1".into(),
        port: Some(port),
        database: "synthetic".into(),
        user: "saya".into(),
        ssl_mode: Some(saya_types::PostgresSslMode::Disable),
        password: None,
    };
    let mut runtime = test_runtime(assisted_memory());
    runtime.resolved.profile_name = Some("setup".into());
    runtime.resolved.profile = Some(profile.clone());
    runtime.resolved.investigation_budgets = ResolvedInvestigationBudgets {
        logical_answering_requests: finite(8),
        requested_tool_calls: finite(8),
        elapsed_seconds: finite(1),
        known_reported_tokens: finite(100),
    };
    runtime.resolved.query_timeout_seconds = 4;
    runtime.connections = ConnectionsFile {
        profiles: BTreeMap::from([("setup".into(), profile)]),
    };
    runtime.connections_path = Some(root.join("connections.toml"));
    runtime.cache_scope = root.to_path_buf();
    runtime.investigations_root = root.join("investigations");
    runtime
}
