//! Config file adapters share bounded reads without changing missing-file or link policy.

use super::{RuntimeError, read_config, read_required_config};
use saya_config::MAX_CONFIG_FILE_BYTES;
use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("saya-config-read-{}-{stamp}", std::process::id()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn both_adapters_reject_oversized_files_before_toml_parsing() {
    let temp = Temp::new();
    let path = temp.0.join("config.toml");
    fs::write(&path, vec![b' '; MAX_CONFIG_FILE_BYTES + 1]).unwrap();
    for result in [
        read_config(&Some(path.clone())).map(|_| ()),
        read_required_config(&path).map(|_| ()),
    ] {
        assert!(
            matches!(result, Err(RuntimeError::Read { source, .. }) if source.kind() == std::io::ErrorKind::InvalidData)
        );
    }
}

#[test]
fn exact_file_bound_and_missing_path_policies_are_preserved() {
    let temp = Temp::new();
    let path = temp.0.join("config.toml");
    assert!(read_config(&Some(path.clone())).unwrap().is_none());
    assert!(matches!(
        read_required_config(&path),
        Err(RuntimeError::Missing(_))
    ));
    fs::write(&path, vec![b'#'; MAX_CONFIG_FILE_BYTES]).unwrap();
    assert!(read_config(&Some(path.clone())).unwrap().is_some());
    assert!(read_required_config(&path).is_ok());
}

#[test]
fn non_utf8_and_non_regular_inputs_are_typed_read_errors() {
    let temp = Temp::new();
    let path = temp.0.join("config.toml");
    fs::write(&path, [0xff, 0xfe]).unwrap();
    assert!(
        matches!(read_required_config(&path), Err(RuntimeError::Read { source, .. }) if source.kind() == std::io::ErrorKind::InvalidData)
    );
    assert!(matches!(
        read_required_config(&temp.0),
        Err(RuntimeError::Read { .. })
    ));
}

#[cfg(unix)]
#[test]
fn a_symlink_to_a_regular_config_still_loads() {
    let temp = Temp::new();
    let target = temp.0.join("target.toml");
    let link = temp.0.join("config.toml");
    fs::write(&target, "[coding]\nenabled = true\n").unwrap();
    std::os::unix::fs::symlink(&target, &link).unwrap();
    assert_eq!(
        read_required_config(&link).unwrap().coding.enabled,
        Some(true)
    );
}

#[cfg(unix)]
#[test]
fn fifo_config_is_refused_without_waiting_for_a_writer() {
    use std::{
        ffi::CString,
        os::unix::ffi::OsStrExt,
        process::{Command, Stdio},
        thread,
        time::{Duration, Instant},
    };
    const CHILD_DIR: &str = "SAYA_TEST_CONFIG_FIFO_CHILD";
    if let Some(dir) = std::env::var_os(CHILD_DIR) {
        let dir = PathBuf::from(dir);
        assert!(matches!(read_required_config(&dir.join("config.toml")),
                Err(RuntimeError::Read { source, .. })
                    if source.kind() == std::io::ErrorKind::InvalidInput));
        fs::write(dir.join("completed"), b"fifo refused").unwrap();
        return;
    }
    let temp = Temp::new();
    let path = temp.0.join("config.toml");
    let name = CString::new(path.as_os_str().as_bytes()).unwrap();
    // SAFETY: valid terminated path, mode contains ordinary permission bits.
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    // Process isolation lets the parent kill and reap even an old blocking reader.
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.args([
        "--exact",
        "config::sources::tests::fifo_config_is_refused_without_waiting_for_a_writer",
        "--test-threads=1",
    ]);
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("SAYA_")
            && !key.to_string_lossy().starts_with("SAYA_TEST_")
        {
            command.env_remove(key);
        }
    }
    let mut child = command
        .env(CHILD_DIR, &temp.0)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let outcome = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            Ok(None) => break Err("FIFO reader exceeded its deadline"),
            Err(_) => break Err("could not poll FIFO reader"),
        }
    };
    if outcome.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    assert!(outcome.expect("FIFO child must finish").success());
    // An incorrect test selector must not pass after running zero child tests.
    assert_eq!(fs::read(temp.0.join("completed")).unwrap(), b"fifo refused");
}
