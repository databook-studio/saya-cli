use std::{
    fs,
    process::Command,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
const CHILD: &str = "SAYA_TEST_CODING_OWNER_CHILD";
pub(crate) fn run_parent(case: &str, test: &str) -> bool {
    if std::env::var(CHILD).ok().as_deref() == Some(case) {
        return false;
    }
    use std::os::unix::fs::PermissionsExt;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "saya-finalization-{case}-{}-{stamp}",
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
        .env(CHILD, case)
        .env("SAYA_STATE_DB", root.join("state.sqlite3"))
        .env("HOME", &root)
        .env("XDG_DATA_HOME", root.join("data"))
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_CACHE_HOME", root.join("cache"));
    use std::os::unix::process::CommandExt;
    for key in [
        "OPENAI_API_KEY",
        "ANTHROPIC_API_KEY",
        "AZURE_OPENAI_API_KEY",
    ] {
        command.env_remove(key);
    }
    let stdout = root.join("child.stdout.log");
    let stderr = root.join("child.stderr.log");
    command
        .process_group(0)
        .stdout(fs::File::create(&stdout).unwrap())
        .stderr(fs::File::create(&stderr).unwrap());
    let mut child = command.spawn().unwrap();
    let deadline = Instant::now() + Duration::from_secs(90);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            // SAFETY: this is the fresh process group created immediately above,
            // never an inferred PID or another test's group.
            unsafe {
                libc::kill(
                    -i32::try_from(child.id()).expect("fresh child PID fits pid_t"),
                    libc::SIGKILL,
                );
            }
            let _ = child.wait();
            panic!(
                "worker child exceeded 90 seconds; evidence at {}",
                root.display()
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(
        status.success(),
        "worker fixture retained at {}\n{}\n{}",
        root.display(),
        fs::read_to_string(stdout).unwrap(),
        fs::read_to_string(stderr).unwrap()
    );
    fs::remove_dir_all(root).unwrap();
    true
}
