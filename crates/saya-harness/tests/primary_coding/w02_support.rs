use std::{
    env,
    ffi::OsString,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

pub struct TempRoot(PathBuf);

impl TempRoot {
    pub fn new(label: &str) -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = env::temp_dir().join(format!(
            "saya-w02-{label}-{}-{stamp}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        Self(fs::canonicalize(path).unwrap())
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

pub fn git_executable() -> PathBuf {
    env::split_paths(&env::var_os("PATH").unwrap())
        .map(|dir| dir.join("git"))
        .find(|path| path.is_file())
        .map(|path| fs::canonicalize(path).unwrap())
        .expect("installed Git")
}

pub fn repository(parent: &Path, executable: &Path) -> PathBuf {
    let root = parent.join("repository with spaces");
    fs::create_dir(&root).unwrap();
    let template = format!("--template={}", parent.join("empty-template").display());
    git_home(parent);
    run_git(
        executable,
        parent,
        git_args(&root, &["init", "--quiet", &template]),
    );
    run_git(
        executable,
        parent,
        git_args(&root, &["config", "user.name", "W02 fixture"]),
    );
    run_git(
        executable,
        parent,
        git_args(&root, &["config", "user.email", "w02@example.invalid"]),
    );
    fs::write(root.join("tracked.txt"), b"base\n").unwrap();
    run_git(
        executable,
        parent,
        git_args(&root, &["add", "--", "tracked.txt"]),
    );
    run_git(
        executable,
        parent,
        git_args(&root, &["commit", "--quiet", "-m", "initial"]),
    );
    root
}

pub fn state_root(parent: &Path) -> PathBuf {
    let root = parent.join("application-state");
    fs::create_dir(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    root
}

pub fn git_args(root: &Path, tail: &[&str]) -> Vec<OsString> {
    let mut args = vec![OsString::from("-C"), root.as_os_str().to_owned()];
    args.extend(tail.iter().map(OsString::from));
    args
}

pub fn run_git(
    executable: &Path,
    fixture_root: &Path,
    args: impl IntoIterator<Item = OsString>,
) -> Vec<u8> {
    let home = git_home(fixture_root);
    let hooks = fixture_root.join("empty-hooks");
    let output = Command::new(executable)
        .env_clear()
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", &home)
        .env("PATH", executable.parent().expect("Git has a parent"))
        .env("TMPDIR", fixture_root)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", home.join("empty.gitconfig"))
        .env("GIT_ATTR_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "")
        .env("SSH_ASKPASS", "")
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .arg("-c")
        .arg(format!("core.hooksPath={}", hooks.display()))
        .args(args)
        .output()
        .expect("Git starts");
    assert!(
        output.status.success(),
        "Git fixture command succeeds: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

pub fn git_home(fixture_root: &Path) -> PathBuf {
    let home = fixture_root.join("git-home");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(fixture_root.join("empty-hooks")).unwrap();
    fs::create_dir_all(fixture_root.join("empty-template")).unwrap();
    fs::write(home.join("empty.gitconfig"), b"").unwrap();
    home
}

pub fn response_git(
    fixture_root: &Path,
    real_git: &Path,
    command: &str,
    response: &[u8],
) -> PathBuf {
    let (match_test, response_name) = match command {
        "status" => (
            "[ \"$status\" -eq 1 ] && [ \"$branch\" -eq 1 ]",
            "status-response",
        ),
        "worktree" => (
            "[ \"$worktree\" -eq 1 ] && [ \"$list\" -eq 1 ]",
            "worktree-response",
        ),
        _ => panic!("only fixed inventory request responses are supported"),
    };
    let response_path = fixture_root.join(response_name);
    fs::write(&response_path, response).unwrap();
    let wrapper = fixture_root.join("git-response-wrapper");
    let response_path = shell_quote(response_path.to_str().expect("fixture path is UTF-8"));
    let real_git = shell_quote(real_git.to_str().expect("Git path is UTF-8"));
    let script = format!(
        "#!/bin/sh\nstatus=0\nbranch=0\nworktree=0\nlist=0\nfor arg do\n  case \"$arg\" in\n    status) status=1 ;;\n    --branch) branch=1 ;;\n    worktree) worktree=1 ;;\n    list) list=1 ;;\n  esac\ndone\nif {match_test}; then\n  exec /bin/cat {response_path}\nfi\nexec {real_git} \"$@\"\n"
    );
    fs::write(&wrapper, script).unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700)).unwrap();
    wrapper
}

pub fn delayed_git(fixture_root: &Path, real_git: &Path) -> (PathBuf, PathBuf, PathBuf) {
    let wrapper = fixture_root.join("git-delayed-wrapper");
    let marker = fixture_root.join("git-dir-request-count");
    let pids = fixture_root.join("git-dir-request-pids");
    let marker_literal = shell_quote(marker.to_str().expect("fixture path is UTF-8"));
    let pids_literal = shell_quote(pids.to_str().expect("fixture path is UTF-8"));
    let real_git = shell_quote(real_git.to_str().expect("Git path is UTF-8"));
    let script = format!(
        "#!/bin/sh\ndelay=0\nfor arg do\n  if [ \"$arg\" = \"--absolute-git-dir\" ]; then delay=1; fi\ndone\nif [ \"$delay\" -eq 1 ]; then\n  printf x >> {marker_literal}\n  /bin/sleep 3 &\n  child=$!\n  printf '%s %s\\n' \"$$\" \"$child\" >> {pids_literal}\n  wait \"$child\"\nfi\nexec {real_git} \"$@\"\n"
    );
    fs::write(&wrapper, script).unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700)).unwrap();
    (wrapper, marker, pids)
}

pub async fn wait_group_gone(pgid: u32) -> bool {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if unsafe { libc::killpg(pgid as libc::pid_t, 0) } != 0
                && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
            {
                break true;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap_or(false)
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
