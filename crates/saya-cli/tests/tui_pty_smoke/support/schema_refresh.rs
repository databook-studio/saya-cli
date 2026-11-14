/// Outcome of a headless `saya connection schema --refresh` run. `success` is
/// the only field the caller branches on; `exit_code` and `stderr` feed the
/// skip message so a clean skip says *why* the database was unusable.
struct SchemaRefreshResult {
    success: bool,
    exit_code: Option<i32>,
    stderr: String,
}

/// Runs `saya connection schema docker_postgres --refresh` headlessly against
/// the scratch HOME, seeding the store's schema cache with a real write
/// against the live pagila database. Sets the docker postgres password in the
/// child env (never inlined in config); never reads or logs it back.
fn run_saya_schema_refresh(home: &Path, config: &ScratchConfig) -> SchemaRefreshResult {
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
        .env(DOCKER_POSTGRES_PASSWORD_ENV, "databook")
        .args([
            "--config",
            config.config.to_str().unwrap(),
            "--connections",
            config.connections.to_str().unwrap(),
            "--approval-mode",
            "read-only",
            "connection",
            "schema",
            "docker_postgres",
            "--refresh",
        ])
        .output()
        .expect("could not spawn saya for schema refresh");

    SchemaRefreshResult {
        success: output.status.success(),
        exit_code: output.status.code(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// Cheap TCP reachability probe with a short deadline. Used only to decide
/// whether rung 3 should run at all — never as an assertion. `127.0.0.1:5434`
/// is the docker pagila port; a refused/timeout connect means the container is
/// not up and the rung skips cleanly.
fn tcp_reachable(addr: &str, timeout: Duration) -> bool {
    use std::net::TcpStream;
    use std::str::FromStr;
    let socket = match std::net::SocketAddr::from_str(addr) {
        Ok(socket) => socket,
        Err(_) => return false,
    };
    TcpStream::connect_timeout(&socket, timeout).is_ok()
}

// ----- assertions that print the captured screen on failure -----------------

fn assert_in(haystack: &str, needle: &str, msg: &str, screen: &str) {
    assert!(
        haystack.contains(needle),
        "{msg}\n--- captured screen ---\n{screen}"
    );
}

fn assert_not_in(haystack: &str, needle: &str, msg: &str, screen: &str) {
    assert!(
        !haystack.contains(needle),
        "{msg}: {needle:?} appeared on screen\n--- captured screen ---\n{screen}"
    );
}
