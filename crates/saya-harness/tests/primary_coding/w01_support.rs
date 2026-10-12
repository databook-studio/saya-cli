use std::{
    env,
    ffi::OsString,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use saya_types::GitObjectId;

static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

pub struct TempRoot(PathBuf);

impl TempRoot {
    pub fn new(label: &str) -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is after epoch")
            .as_nanos();
        let root = env::temp_dir().join(format!(
            "saya-w01-{label}-{}-{stamp}-{}",
            std::process::id(),
            NEXT_ROOT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).expect("temporary root is new");
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700))
            .expect("temporary root is private");
        Self(fs::canonicalize(root).expect("temporary root resolves canonically"))
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub struct RepoFixture {
    pub root: PathBuf,
    pub home: PathBuf,
    pub base: GitObjectId,
}

pub fn git_executable() -> PathBuf {
    let path = env::var_os("PATH").expect("test process has PATH");
    for directory in env::split_paths(&path) {
        let candidate = directory.join("git");
        if candidate.is_file() && candidate.is_absolute() {
            return fs::canonicalize(candidate).expect("installed Git path resolves");
        }
    }
    panic!("installed Git is required for W01 integration tests");
}

pub fn setup_repo(parent: &Path, git: &Path) -> RepoFixture {
    let repo = parent.join("source repo");
    fs::create_dir(&repo).expect("repository directory is new");
    let home = setup_home(parent, "setup-home");
    run_git(
        git,
        &home,
        args(&["init", "--quiet", repo.to_str().unwrap()]),
    );
    run_git(
        git,
        &home,
        git_args(&repo, &["config", "user.name", "W01 test"]),
    );
    run_git(
        git,
        &home,
        git_args(&repo, &["config", "user.email", "w01@example.invalid"]),
    );
    fs::write(repo.join("tracked.txt"), b"base content\n").expect("fixture file writes");
    run_git(git, &home, git_args(&repo, &["add", "--", "tracked.txt"]));
    run_git(
        git,
        &home,
        git_args(&repo, &["commit", "--quiet", "-m", "initial"]),
    );
    let base = run_git(
        git,
        &home,
        git_args(&repo, &["rev-parse", "--verify", "HEAD^{commit}"]),
    );
    let base = String::from_utf8(base).expect("commit ID is UTF-8");
    let base = GitObjectId::parse(base.trim()).expect("full Git object ID");
    RepoFixture {
        root: repo,
        home,
        base,
    }
}

pub fn current_commit(git: &Path, home: &Path, repo: &Path) -> GitObjectId {
    let output = run_git(
        git,
        home,
        git_args(repo, &["rev-parse", "--verify", "HEAD^{commit}"]),
    );
    let output = String::from_utf8(output).expect("commit ID is UTF-8");
    GitObjectId::parse(output.trim()).expect("full Git object ID")
}

pub fn setup_home(parent: &Path, name: &str) -> PathBuf {
    let home = parent.join(name);
    fs::create_dir(&home).expect("test home is new");
    fs::write(home.join("empty.gitconfig"), b"").expect("empty config writes");
    home
}

pub fn args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

pub fn git_args(repo: &Path, values: &[&str]) -> Vec<OsString> {
    let mut args = vec![OsString::from("-C"), repo.as_os_str().to_owned()];
    args.extend(values.iter().map(OsString::from));
    args
}

pub fn run_git(git: &Path, home: &Path, args: impl IntoIterator<Item = OsString>) -> Vec<u8> {
    let output = git_command(git, home)
        .args(args)
        .output()
        .expect("Git starts");
    assert!(
        output.status.success(),
        "temporary Git setup command succeeds"
    );
    output.stdout
}

pub fn try_git(git: &Path, home: &Path, args: impl IntoIterator<Item = OsString>) -> Output {
    git_command(git, home)
        .args(args)
        .output()
        .expect("Git starts")
}

pub fn git_command(git: &Path, home: &Path) -> Command {
    let mut command = Command::new(git);
    command
        .env_clear()
        .env("HOME", home)
        .env("PATH", git.parent().expect("Git has a parent directory"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", home.join("empty.gitconfig"))
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C");
    command
}

/// Writes an executable helper that is safe to exec immediately. A thread that
/// forks while the file is open for writing leaves its child holding a write
/// descriptor until that child execs, and exec of the file then fails with
/// ETXTBSY; on Linux this waits until no process holds the file for writing.
pub fn write_executable(path: &Path, body: &[u8]) {
    fs::write(path, body).expect("temporary helper writes");
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .expect("temporary helper is executable");
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while held_for_writing(path) && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[cfg(target_os = "linux")]
fn held_for_writing(path: &Path) -> bool {
    let Ok(processes) = fs::read_dir("/proc") else {
        return false;
    };
    processes.flatten().any(|process| {
        let fds = process.path().join("fd");
        fs::read_dir(&fds).is_ok_and(|entries| {
            entries.flatten().any(|fd| {
                fs::read_link(fd.path()).is_ok_and(|target| target == path)
                    && fs::read_to_string(process.path().join("fdinfo").join(fd.file_name()))
                        .ok()
                        .and_then(|info| {
                            let flags =
                                info.lines().find_map(|line| line.strip_prefix("flags:"))?;
                            u32::from_str_radix(flags.trim(), 8).ok()
                        })
                        .is_some_and(|flags| flags & 0o3 != 0)
            })
        })
    })
}

#[cfg(not(target_os = "linux"))]
fn held_for_writing(_path: &Path) -> bool {
    false
}

pub fn trap_script(directory: &Path, name: &str) -> PathBuf {
    let script = directory.join(name);
    let marker = marker_for(&script);
    let marker = marker
        .to_str()
        .expect("temporary path is UTF-8")
        .replace('\'', "'\\''");
    write_executable(
        &script,
        format!("#!/bin/sh\nprintf ran > '{marker}'\n").as_bytes(),
    );
    script
}

pub fn marker_for(script: &Path) -> PathBuf {
    let mut marker = script.as_os_str().to_owned();
    marker.push(".ran");
    PathBuf::from(marker)
}

pub fn fake_git(directory: &Path, body: &str) -> (PathBuf, PathBuf) {
    let executable = directory.join("git-probe");
    write_executable(&executable, body.as_bytes());
    let mut marker = executable.as_os_str().to_owned();
    marker.push(".pids");
    (executable, PathBuf::from(marker))
}

pub fn process_is_alive(pid: u32) -> bool {
    let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
    result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

pub fn wait_for_pids(marker: &Path) -> Vec<u32> {
    for _ in 0..200 {
        if let Ok(contents) = fs::read_to_string(marker) {
            let pids = contents
                .split_whitespace()
                .filter_map(|value| value.parse().ok())
                .collect::<Vec<u32>>();
            if !pids.is_empty() {
                return pids;
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("test Git process did not record its process IDs");
}

pub fn assert_processes_gone(pids: &[u32]) {
    for _ in 0..400 {
        if pids.iter().all(|pid| !process_is_alive(*pid)) {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        pids.iter().all(|pid| !process_is_alive(*pid)),
        "process group is reaped"
    );
}
