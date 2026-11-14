use std::{
    fs,
    os::unix::{fs::PermissionsExt, process::CommandExt},
    path::PathBuf,
    process::Command,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
pub const CHILD: &str = "SAYA_PARITY_PROBE_CHILD";
pub const ROOT: &str = "SAYA_PARITY_PROBE_ROOT";

pub fn isolated(test: &str) -> bool {
    if std::env::var_os(CHILD).is_some() {
        return false;
    }
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "saya-probe-resource-{}-{stamp}",
        std::process::id()
    ));
    fs::create_dir(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    let root = root.canonicalize().unwrap();
    let mut command = Command::new(std::env::current_exe().unwrap());
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("SAYA_") {
            command.env_remove(key);
        }
    }
    command
        .args(["--exact", test, "--nocapture"])
        .env(CHILD, "1")
        .env(ROOT, &root)
        .process_group(0)
        .stdout(fs::File::create(root.join("child.stdout.log")).unwrap())
        .stderr(fs::File::create(root.join("child.stderr.log")).unwrap());
    let mut child = command.spawn().unwrap();
    let deadline = Instant::now() + Duration::from_secs(180);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            // SAFETY: this is exactly the fresh process group created above.
            unsafe {
                libc::kill(-i32::try_from(child.id()).unwrap(), libc::SIGKILL);
            }
            let _ = child.wait();
            panic!(
                "fixture deadline exceeded; evidence preserved at {}",
                root.display()
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(
        status.success(),
        "probe resource owner failed; evidence at {}:\n{}\n{}",
        root.display(),
        fs::read_to_string(root.join("child.stdout.log")).unwrap(),
        fs::read_to_string(root.join("child.stderr.log")).unwrap()
    );
    fs::remove_dir_all(root).unwrap();
    true
}

pub fn root() -> PathBuf {
    PathBuf::from(std::env::var_os(ROOT).unwrap())
}

pub fn thread_count() -> usize {
    let pid = std::process::id().to_string();
    let output = Command::new("/bin/ps")
        .args(["-M", "-p", &pid, "-o", "pid="])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "fixture unavailable: ps thread observation failed"
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let mut rows = stdout.lines().filter(|line| !line.trim().is_empty());
    let header = rows.next().unwrap_or_default();
    assert_eq!(
        header.split_whitespace().take(2).collect::<Vec<_>>(),
        ["USER", "PID"],
        "fixture unavailable: unexpected actual ps header: {stdout:?}"
    );
    // macOS -M retains its thread columns and appends the requested pid field.
    let rows = rows.collect::<Vec<_>>();
    assert!(
        !rows.is_empty()
            && rows.len() <= 128
            && rows
                .iter()
                .all(|row| row.split_whitespace().last() == Some(pid.as_str())),
        "fixture unavailable: unexpected actual ps shape: {stdout:?}"
    );
    rows.len()
}
