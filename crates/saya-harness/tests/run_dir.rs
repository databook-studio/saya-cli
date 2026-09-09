use std::path::{Path, PathBuf};

use saya_harness::{HarnessError, lock::RunLock, paths::resolve_runs_dir, run_dir::RunDir};
use saya_types::RunId;

fn temp_root(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!("saya-harness-{label}-{}", std::process::id()))
}

/// A pid no real process can have on any supported platform: Linux caps pids
/// well below 2^22 (`/proc/sys/kernel/pid_max`) and macOS well below 10^5, so
/// this value is parseable but always names a dead process.
const DEAD_PID: u32 = i32::MAX as u32 - 1;

#[test]
fn run_dir_creates_run_workspace_and_state_at_0700() {
    let runs_root = temp_root("run-dir");
    let id = RunId::parse("run-1").unwrap();
    let run_dir = RunDir::create(&runs_root, &id).unwrap();

    assert_eq!(run_dir.root(), runs_root.join("run-1"));
    assert_eq!(run_dir.workspace(), runs_root.join("run-1/workspace"));
    assert_eq!(run_dir.state(), runs_root.join("run-1/state"));
    assert!(run_dir.workspace().is_dir());
    assert!(run_dir.state().is_dir());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for dir in [
            runs_root.as_path(),
            run_dir.root(),
            run_dir.workspace(),
            run_dir.state(),
        ] {
            assert_eq!(
                std::fs::metadata(dir).unwrap().permissions().mode() & 0o777,
                0o700,
                "{}",
                dir.display()
            );
        }
    }
    let _ = std::fs::remove_dir_all(runs_root);
}

#[test]
fn run_dir_is_idempotent_and_repairs_permissions() {
    let runs_root = temp_root("run-dir-repair");
    let id = RunId::parse("run-2").unwrap();
    let first = RunDir::create(&runs_root, &id).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let loose = std::fs::Permissions::from_mode(0o755);
        std::fs::set_permissions(first.root(), loose.clone()).unwrap();
        std::fs::set_permissions(first.workspace(), loose).unwrap();
    }
    #[cfg(not(unix))]
    {
        let _ = &first;
    }
    let second = RunDir::create(&runs_root, &id).unwrap();
    assert_eq!(second.root(), first.root());
    assert!(second.workspace().is_dir());
    assert!(second.state().is_dir());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for dir in [second.root(), second.workspace(), second.state()] {
            assert_eq!(
                std::fs::metadata(dir).unwrap().permissions().mode() & 0o777,
                0o700,
                "{:?}",
                dir
            );
        }
    }
    let _ = std::fs::remove_dir_all(runs_root);
}

#[test]
fn runs_dir_resolution_prefers_override_then_xdg_then_appdata_then_home() {
    assert_eq!(
        resolve_runs_dir(Some("/runs"), None, None, None),
        Path::new("/runs")
    );
    assert_eq!(
        resolve_runs_dir(None, Some("/xdg"), None, None),
        Path::new("/xdg/saya/runs")
    );
    assert_eq!(
        resolve_runs_dir(None, None, Some("/appdata"), None),
        Path::new("/appdata/saya/runs")
    );
    assert_eq!(
        resolve_runs_dir(None, None, None, Some("/home")),
        Path::new("/home/.local/share/saya/runs")
    );
    assert_eq!(
        resolve_runs_dir(None, None, None, None),
        Path::new(".local/share/saya/runs")
    );
}

#[test]
fn lock_refuses_a_second_holder_while_the_first_is_alive() {
    let dir = temp_root("lock-live");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("lock");

    let holder = RunLock::acquire(&path).unwrap();
    assert_eq!(holder.pid(), std::process::id());
    assert!(path.is_file());

    let refused = RunLock::acquire(&path).expect_err("a live holder must refuse");
    match refused {
        HarnessError::LockHeld { pid } => assert_eq!(pid, std::process::id()),
        other => panic!("expected LockHeld, got {other:?}"),
    }

    holder.release().unwrap();
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
#[cfg(unix)]
fn lock_reclaims_a_stale_holder() {
    let dir = temp_root("lock-stale");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("lock");
    std::fs::write(&path, format!("{DEAD_PID}\n")).unwrap();

    let lock = RunLock::acquire(&path).unwrap();
    assert_eq!(lock.pid(), std::process::id());
    assert_eq!(
        std::fs::read_to_string(&path).unwrap().trim(),
        lock.pid().to_string()
    );
    lock.release().unwrap();
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
#[cfg(unix)]
fn lock_reclaims_unreadable_lock_files() {
    for content in ["", "not a pid\n"] {
        let dir = temp_root("lock-junk");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("lock");
        std::fs::write(&path, content).unwrap();

        let lock = RunLock::acquire(&path).unwrap();
        assert_eq!(lock.pid(), std::process::id());
        lock.release().unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[test]
fn lock_release_removes_the_file_and_allows_reacquire() {
    let dir = temp_root("lock-release");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("lock");

    let lock = RunLock::acquire(&path).unwrap();
    lock.release().unwrap();
    assert!(!path.exists());

    let again = RunLock::acquire(&path).unwrap();
    assert_eq!(again.pid(), std::process::id());
    again.release().unwrap();
    let _ = std::fs::remove_dir_all(dir);
}
