//! The managed runtime operator command is a real CLI boundary. The no-consent
//! path must not resolve repository/provider configuration or create state.

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_TEMP_ROOT: AtomicU64 = AtomicU64::new(0);

struct TempRoot(PathBuf);

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn temp_root() -> TempRoot {
    for _ in 0..100 {
        let root = env::temp_dir().join(format!(
            "saya-node-install-refusal-{}-{}",
            std::process::id(),
            NEXT_TEMP_ROOT.fetch_add(1, Ordering::Relaxed)
        ));
        match fs::create_dir(&root) {
            Ok(()) => {
                let root = TempRoot(root);
                fs::create_dir(root.0.join("launch")).unwrap();
                fs::create_dir(root.0.join("home")).unwrap();
                fs::create_dir_all(root.0.join("config-home/saya")).unwrap();
                return root;
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => panic!("create temporary test root: {error}"),
        }
    }
    panic!("could not claim a unique temporary test root after 100 attempts")
}

fn tree_paths(root: &Path) -> Vec<PathBuf> {
    fn append(root: &Path, directory: &Path, paths: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            paths.push(path.strip_prefix(root).unwrap().to_path_buf());
            if entry.file_type().unwrap().is_dir() {
                append(root, &path, paths);
            }
        }
    }

    let mut paths = Vec::new();
    append(root, root, &mut paths);
    paths.sort();
    paths
}

#[test]
fn node_install_without_yes_is_an_early_no_effect_refusal() {
    let root = temp_root();
    let launch = root.0.join("launch");
    let config = root.0.join("config-home/saya/config.toml");
    let invalid_config = b"not = [valid TOML";
    fs::write(&config, invalid_config).unwrap();
    assert!(
        !launch.join(".git").exists(),
        "the launch directory is not a repository"
    );

    let state_db = root.0.join("state.sqlite3");
    let before = tree_paths(&root.0);
    let mut command = Command::new(env!("CARGO_BIN_EXE_saya"));
    command
        .args(["--non-interactive", "runtime", "install-node"])
        .current_dir(&launch)
        .env_clear()
        .env("HOME", root.0.join("home"))
        .env("SAYA_CONFIG_HOME", root.0.join("config-home"))
        .env("SAYA_STATE_DB", &state_db)
        .env("PATH", "")
        .env("LANG", "C")
        .env("LC_ALL", "C");
    for (name, value) in
        env::vars_os().filter(|(name, _)| name.to_string_lossy().starts_with("SAYA_TEST_"))
    {
        command.env(name, value);
    }
    let output = command.output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let message = format!("{stdout}{stderr}");

    assert_eq!(
        output.status.code(),
        Some(2),
        "missing --yes is refused before installation: {message}"
    );
    assert!(
        message.contains("--yes"),
        "the refusal names the explicit consent flag: {message}"
    );
    assert_eq!(
        fs::read(&config).unwrap(),
        invalid_config,
        "trusted user configuration remains untouched and is not parsed"
    );
    assert!(
        !state_db.exists(),
        "the state database is not opened or created"
    );
    assert_eq!(
        tree_paths(&root.0),
        before,
        "no runtime registry or other state is created without consent"
    );
}

#[test]
fn node_install_refuses_session_only_launch_flags() {
    let root = temp_root();
    let launch = root.0.join("launch");
    let mut command = Command::new(env!("CARGO_BIN_EXE_saya"));
    command
        .args([
            "--allow",
            "workspace-write",
            "runtime",
            "install-node",
            "--yes",
        ])
        .current_dir(&launch)
        .env_clear()
        .env("HOME", root.0.join("home"))
        .env("SAYA_STATE_DB", root.0.join("state.sqlite3"))
        .env("PATH", "")
        .env("LANG", "C")
        .env("LC_ALL", "C");
    for (name, value) in
        env::vars_os().filter(|(name, _)| name.to_string_lossy().starts_with("SAYA_TEST_"))
    {
        command.env(name, value);
    }
    let output = command.output().unwrap();
    let message = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    assert_eq!(
        output.status.code(),
        Some(2),
        "a session-only flag cannot be silently ignored: {message}"
    );
    assert!(
        message.contains("--allow") && message.contains("interactive session"),
        "the refusal explains that --allow belongs to the interactive session: {message}"
    );
}

#[test]
fn node_install_rejects_unusable_state_paths_before_runtime_effects() {
    use std::ffi::OsString;

    let root = temp_root();
    let launch = root.0.join("launch");
    let config = root.0.join("config-home/saya/config.toml");
    let invalid_config = b"not = [valid TOML";
    fs::write(&config, invalid_config).unwrap();
    let directory = root.0.join("state-directory");
    fs::create_dir_all(&directory).unwrap();
    let directory_with_separator = OsString::from(format!(
        "{}{separator}",
        directory.display(),
        separator = std::path::MAIN_SEPARATOR
    ));
    let state_paths = [
        ("empty override", OsString::new()),
        (
            "filesystem root",
            OsString::from(std::path::MAIN_SEPARATOR.to_string()),
        ),
        ("directory override", directory_with_separator),
    ];
    let before = tree_paths(&root.0);

    for (label, state_path) in state_paths {
        let mut command = Command::new(env!("CARGO_BIN_EXE_saya"));
        command
            .args(["--non-interactive", "runtime", "install-node", "--yes"])
            .current_dir(&launch)
            .env_clear()
            .env("HOME", root.0.join("home"))
            .env("SAYA_CONFIG_HOME", root.0.join("config-home"))
            .env("SAYA_STATE_DB", &state_path)
            .env("PATH", "")
            .env("LANG", "C")
            .env("LC_ALL", "C");
        for (name, value) in
            env::vars_os().filter(|(name, _)| name.to_string_lossy().starts_with("SAYA_TEST_"))
        {
            command.env(name, value);
        }
        let output = command.output().unwrap();
        let message = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        assert_eq!(
            output.status.code(),
            Some(3),
            "{label} is refused before installer execution: {message}"
        );
        assert!(
            message.contains("configured state database path"),
            "{label} yields a state-path diagnostic before config/provider work: {message}"
        );
        assert_eq!(
            fs::read(&config).unwrap(),
            invalid_config,
            "{label} does not resolve or rewrite trusted config"
        );
        assert_eq!(
            tree_paths(&root.0),
            before,
            "{label} does not create state or runtime files"
        );
    }
}

#[path = "runtime_installation/npm_actions.rs"]
mod npm_actions;
#[cfg(unix)]
#[path = "runtime_installation/registry_binding.rs"]
mod registry_binding;
