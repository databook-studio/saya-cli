use super::state_root_from_db_path;
#[cfg(unix)]
use super::{CancellationToken, InstallStop, completed_operator_result, settle_install};
#[cfg(unix)]
use std::{
    env,
    path::PathBuf,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use std::{
    fs,
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(unix)]
struct MarkerRoot(PathBuf);

#[cfg(unix)]
impl Drop for MarkerRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn existing_directory_without_trailing_separator_is_not_a_database_path() {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let directory = std::env::temp_dir().join(format!(
        "saya-runtime-state-directory-{}-{stamp}",
        std::process::id()
    ));
    fs::create_dir(&directory).expect("create directory fixture");
    let rendered = directory.as_os_str().to_string_lossy();
    assert!(directory.is_dir());
    assert!(!rendered.ends_with(std::path::MAIN_SEPARATOR));

    let parent = state_root_from_db_path(&directory);
    assert!(
        parent.is_err(),
        "an existing directory is not a database file"
    );

    #[cfg(unix)]
    {
        let alias = directory.with_extension("directory-symlink");
        std::os::unix::fs::symlink(&directory, &alias).expect("create directory symlink");
        assert!(
            state_root_from_db_path(&alias).is_err(),
            "a symlink to an existing directory is not a database file"
        );
        let _ = fs::remove_file(alias);
    }
    let _ = fs::remove_dir(&directory);
}

#[cfg(unix)]
#[test]
fn sigterm_waits_for_runtime_operation_settlement() {
    let root = MarkerRoot(marker_root());
    fs::create_dir(&root.0).expect("create signal-test directory");
    let ready = root.0.join("ready");
    let settled = root.0.join("settled");
    let mut child = Command::new(env::current_exe().expect("test executable"))
        .args([
            "--exact",
            "commands::runtime::tests::sigterm_child_waits_for_settlement",
            "--nocapture",
        ])
        .env_remove("SAYA_TEST_RUNTIME_READY")
        .env_remove("SAYA_TEST_RUNTIME_SETTLED")
        .env("SAYA_TEST_RUNTIME_READY", &ready)
        .env("SAYA_TEST_RUNTIME_SETTLED", &settled)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn signal-test child");

    let ready_deadline = Instant::now() + Duration::from_secs(5);
    while !ready.exists() {
        if let Some(status) = child.try_wait().expect("poll child") {
            panic!("child exited before operation readiness: {status}");
        }
        if Instant::now() >= ready_deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("child did not reach its pending operation");
        }
        thread::sleep(Duration::from_millis(10));
    }

    let signal_result = unsafe { libc::kill(child.id() as libc::pid_t, libc::SIGTERM) };
    if signal_result != 0 {
        let _ = child.kill();
        let _ = child.wait();
        panic!("could not send SIGTERM to the test child");
    }
    let settle_deadline = Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(status) = child.try_wait().expect("poll child settlement") {
            break status;
        }
        if Instant::now() >= settle_deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("signal owner did not settle the child operation in time");
        }
        thread::sleep(Duration::from_millis(10));
    };
    let settled_marker = fs::read(&settled);

    assert_eq!(
        status.code(),
        Some(143),
        "SIGTERM is reported after the cancellation owner settles"
    );
    let marker = settled_marker.expect("settled receipt marker written before exit");
    let marker = String::from_utf8(marker).expect("rendered result is UTF-8");
    assert!(marker.contains("completed receipt"));
}

#[cfg(unix)]
#[test]
fn sigterm_child_waits_for_settlement() {
    let (Some(ready), Some(settled)) = (
        env::var_os("SAYA_TEST_RUNTIME_READY"),
        env::var_os("SAYA_TEST_RUNTIME_SETTLED"),
    ) else {
        return;
    };
    let cancellation = CancellationToken::new();
    let operation_cancellation = cancellation.clone();
    let work = async move {
        fs::write(ready, b"ready").expect("write readiness marker");
        operation_cancellation.cancelled().await;
        "completed receipt"
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build signal-test runtime");
    let (receipt, stop) = runtime
        .block_on(settle_install(&cancellation, work))
        .expect("register runtime stop signals");
    assert_eq!(stop, InstallStop::Terminate);
    crate::commands::capture_output_start();
    let exit_code =
        completed_operator_result(receipt.to_owned(), stop, crate::render::RenderFormat::Text)
            .expect("render settled signal result");
    let (_, stderr) = crate::commands::capture_output_take();
    fs::write(settled, stderr).expect("write settled receipt marker");
    std::process::exit(exit_code);
}

#[cfg(unix)]
fn marker_root() -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "saya-runtime-signal-{}-{stamp}",
        std::process::id()
    ))
}

#[cfg(unix)]
#[test]
fn stop_signal_overrides_success_and_failure_codes() {
    assert_eq!(super::stop_code_for(InstallStop::Completed, 0), 0);
    assert_eq!(super::stop_code_for(InstallStop::Completed, 4), 4);
    assert_eq!(super::stop_code_for(InstallStop::Interrupt, 0), 130);
    assert_eq!(super::stop_code_for(InstallStop::Interrupt, 4), 130);
    assert_eq!(super::stop_code_for(InstallStop::Terminate, 0), 143);
    assert_eq!(super::stop_code_for(InstallStop::Terminate, 4), 143);
}
