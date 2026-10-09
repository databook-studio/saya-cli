use super::isolated_root;
use std::{
    fs,
    io::Write as _,
    path::Path,
    time::{Duration, Instant},
};

/// Drives `saya setup` on a real pty with scripted answer lines, asserts the
/// flow exits 0, and returns the captured output. Unix-only: pty allocation;
/// the headless paths above cover the rest.
#[cfg(not(windows))]
fn drive_setup_pty(root: &Path, lines: &[&str]) -> String {
    use portable_pty::{CommandBuilder, PtySize, native_pty_system};

    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: 40,
            cols: 100,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("pty allocates");

    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_saya"));
    cmd.env("SAYA_CONFIG_HOME", root.join("config-home"));
    cmd.env("SAYA_STATE_DB", root.join("state.sqlite3"));
    cmd.env("HOME", root);
    cmd.env_remove("XDG_CONFIG_HOME");
    cmd.env_remove("XDG_DATA_HOME");
    cmd.env_remove("APPDATA");
    cmd.cwd(root);
    cmd.arg("setup");
    let mut child = pair
        .slave
        .spawn_command(cmd)
        .expect("child spawns on the pty");
    drop(pair.slave);

    // Feed the whole script up front: the flow reads line by line and the pty
    // buffers input, so no answer timing is needed.
    let mut writer = pair.master.take_writer().unwrap();
    for line in lines {
        writer.write_all(line.as_bytes()).unwrap();
        writer.write_all(b"\n").unwrap();
        writer.flush().unwrap();
    }
    drop(writer);

    // Pump output on a reader thread; the main loop waits for the child to
    // exit and the stream to go quiet, with a hard deadline so CI never hangs.
    let reader = pair.master.try_clone_reader().unwrap();
    let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut reader = reader;
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut output = Vec::new();
    let mut quiet_since: Option<Instant> = None;
    loop {
        assert!(
            Instant::now() < deadline,
            "setup never finished on the pty; captured:\n{}",
            String::from_utf8_lossy(&output)
        );
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(chunk) => {
                output.extend_from_slice(&chunk);
                quiet_since = None;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if child.try_wait().unwrap().is_some() {
                    let quiet = quiet_since.get_or_insert_with(Instant::now);
                    if quiet.elapsed() > Duration::from_millis(500) {
                        break;
                    }
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    let status = child.wait().unwrap();
    let text = String::from_utf8_lossy(&output).into_owned();
    assert_eq!(status.exit_code(), 0, "the flow succeeds: {text}");
    text
}

/// The full guided flow through a real pty: skip the provider, add a sqlite
/// profile, watch the probe pass, confirm — files written, exit 0, no marker.
/// 6 = skip provider, 1 = sqlite, path, profile name, y = confirm the write.
#[test]
#[cfg(not(windows))]
fn setup_full_sqlite_flow_through_a_pty() {
    let root = isolated_root("pty");
    // An empty file is a valid empty SQLite database, so the probe succeeds.
    let db_file = root.join("team.db");
    fs::File::create(&db_file).unwrap();

    let text = drive_setup_pty(&root, &["6", "1", db_file.to_str().unwrap(), "team", "y"]);
    assert!(
        text.contains("database reachable"),
        "the probe label: {text}"
    );
    assert!(
        text.contains("configuration valid"),
        "the commit label: {text}"
    );
    assert!(
        text.contains("saya --profile team"),
        "the next step: {text}"
    );

    let connections = root.join("config-home/saya/connections.toml");
    let written = fs::read_to_string(&connections).unwrap();
    assert!(
        written.contains("[profiles.team]"),
        "the profile is written: {written}"
    );
    assert!(
        written.contains(db_file.to_str().unwrap()),
        "the path is kept: {written}"
    );
    assert!(
        !root.join("config-home/saya/.setup-commit.json").exists(),
        "no marker is left"
    );
    let _ = fs::remove_dir_all(&root);
}

/// The ClickHouse guided flow through a real pty: TLS yes, defaults for the
/// port, database and user skipped, the password only as an env-var name.
/// 6 = skip provider, 6 = clickhouse. The password env is unset in the child,
/// so the probe fails fast at secret resolution — a failed probe does not
/// block writing.
#[test]
#[cfg(not(windows))]
fn setup_full_clickhouse_flow_through_a_pty() {
    let root = isolated_root("pty-clickhouse");

    let text = drive_setup_pty(
        &root,
        &[
            "6",
            "6",
            "localhost",
            "y",
            "",
            "",
            "",
            "SAYA_CH_PASSWORD",
            "ch",
            "y",
        ],
    );
    assert!(
        text.contains("Probing the database"),
        "the probe line: {text}"
    );

    let connections = root.join("config-home/saya/connections.toml");
    let written = fs::read_to_string(&connections).unwrap();
    assert!(
        written.contains("[profiles.ch]"),
        "the profile is written: {written}"
    );
    let parsed = saya_config::ConnectionsFile::from_toml(&written)
        .unwrap_or_else(|error| panic!("the written file parses: {error}"));
    assert_eq!(
        parsed.profiles.get("ch"),
        Some(&saya_types::DatabaseProfile::ClickHouse {
            host: "localhost".into(),
            port: None,
            database: None,
            user: None,
            password: Some(saya_types::SecretRef::Env {
                env: "SAYA_CH_PASSWORD".into()
            }),
            secure: Some(true),
        }),
        "the guided flow wrote the expected typed profile: {written}"
    );
    assert!(
        !root.join("config-home/saya/.setup-commit.json").exists(),
        "no marker is left"
    );
    let _ = fs::remove_dir_all(&root);
}
