//! Fixture support for the symlink snapshot contracts: a private temp root,
//! a local Git repository with an isolated environment, and the state root
//! the Git service is opened against.

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
            "saya-symlink-{label}-{}-{stamp}-{}",
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
        git_args(&root, &["config", "user.name", "Symlink fixture"]),
    );
    run_git(
        executable,
        parent,
        git_args(&root, &["config", "user.email", "symlink@example.invalid"]),
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

fn git_home(fixture_root: &Path) -> PathBuf {
    let home = fixture_root.join("git-home");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(fixture_root.join("empty-hooks")).unwrap();
    fs::create_dir_all(fixture_root.join("empty-template")).unwrap();
    fs::write(home.join("empty.gitconfig"), b"").unwrap();
    home
}
