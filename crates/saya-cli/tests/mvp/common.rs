use std::{
    fs,
    path::Path,
    path::PathBuf,
    process::{Command as ProcessCommand, Output},
};

pub(crate) fn test_root(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("saya-cli-{label}-{}", std::process::id()));
    if root.exists() {
        let _ = fs::remove_dir_all(&root);
    }
    fs::create_dir_all(&root).unwrap();
    root
}

/// Creates a SQLite database file and runs `statements` against it through
/// sqlx — direct, not through saya — for tests that need a real file database.
pub(crate) fn sqlite_fixture(path: &Path, statements: &[&str]) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let options = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true);
        let pool = sqlx::SqlitePool::connect_with(options).await.unwrap();
        for statement in statements {
            sqlx::query(statement).execute(&pool).await.unwrap();
        }
        pool.close().await;
    });
}

/// The host variables a spawned `saya` may inherit: operating-system plumbing
/// only. Every other variable, above all each `SAYA_*` setting and API key a
/// developer exports in their shell, is dropped so the host cannot change what
/// a test observes. A test sets what it needs on the returned command.
const INHERITED_HOST_ENV: &[&str] = &[
    "PATH",
    "HOME",
    "TMPDIR",
    "TEMP",
    "TMP",
    "SystemRoot",
    "USERPROFILE",
];

/// The `saya` binary under test with an allowlisted environment: nothing is
/// inherited from the test runner except [`INHERITED_HOST_ENV`]. Build every
/// spawned `saya` here rather than with `Command::new(env!("CARGO_BIN_EXE_saya"))`,
/// which would hand the child the developer's whole environment.
pub(crate) fn saya_binary() -> ProcessCommand {
    let mut command = ProcessCommand::new(env!("CARGO_BIN_EXE_saya"));
    command.env_clear();
    for name in INHERITED_HOST_ENV {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    command
}

pub(crate) fn saya_process(root: &Path, args: &[&str]) -> Output {
    saya_command(root, args).output().unwrap()
}

pub(crate) fn saya_command(root: &Path, args: &[&str]) -> ProcessCommand {
    let home = root.join("home");
    let config = root.join("user-config");
    let sessions = root.join("sessions");
    let cwd = root.join("cwd");
    for path in [&home, &config, &sessions, &cwd] {
        fs::create_dir_all(path).unwrap();
    }
    let mut command = saya_binary();
    command
        .args(args)
        .current_dir(cwd)
        .env("HOME", home)
        .env("SAYA_CONFIG_HOME", config)
        .env("SAYA_SESSION_DIR", sessions)
        .env("SAYA_STATE_DB", root.join("state.sqlite3"));
    command
}

pub(crate) fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

pub(crate) fn run_cli(global: &[&str], command: &[&str], state: &Path) -> Output {
    saya_binary()
        .args(global)
        .args(command)
        .env("SAYA_STATE_DB", state)
        .output()
        .unwrap()
}

/// Reads one HTTP request from a mock's accepted connection to completion:
/// the headers, then exactly the `Content-Length` body bytes. The scripted
/// mocks must answer only after the whole request is drained — reqwest sends
/// the body after the headers, and a single short read leaves the socket with
/// unread bytes, so closing it resets the connection mid-request and the
/// client retries, consuming the scripted responses.
pub(crate) fn drain_request(stream: &mut std::net::TcpStream) {
    use std::io::Read;
    let (mut buffer, mut chunk) = (Vec::new(), [0_u8; 8192]);
    loop {
        let head = buffer.windows(4).position(|window| window == b"\r\n\r\n");
        if let Some(head) = head {
            let length = String::from_utf8_lossy(&buffer[..head])
                .lines()
                .find_map(|line| match line.split_once(':') {
                    Some((name, value)) if name.eq_ignore_ascii_case("content-length") => {
                        value.trim().parse::<usize>().ok()
                    }
                    _ => None,
                })
                .unwrap_or(0);
            if buffer.len() >= head + 4 + length {
                return;
            }
        }
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => return,
            Ok(n) => buffer.extend_from_slice(&chunk[..n]),
        }
    }
}
