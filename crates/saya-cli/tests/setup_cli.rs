//! Binary tests for `saya setup` (S16): the non-interactive refusal (exit 2,
//! stdin untouched), the interrupted-setup startup warning on every other
//! command, completions staying silent, and one full guided flow through a
//! real pty. Every run is isolated to a scratch config home; nothing touches
//! the developer's files.

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

fn isolated_root(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("saya-cli-setup-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

/// The binary pointed at the scratch root, launched from it too, so no
/// project layer and no real user config can interfere.
fn saya_command(root: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_saya"));
    cmd.current_dir(root)
        .env("SAYA_CONFIG_HOME", root.join("config-home"))
        .env("SAYA_STATE_DB", root.join("state.sqlite3"))
        .env("HOME", root)
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_DATA_HOME")
        .env_remove("APPDATA");
    cmd
}

fn stderr_of(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Writes a valid pending-commit marker (one appended connections.toml entry)
/// into the scratch user config directory.
fn stage_marker(root: &Path) -> PathBuf {
    let user_dir = root.join("config-home").join("saya");
    fs::create_dir_all(&user_dir).unwrap();
    let marker = serde_json::json!({
        "version": 1,
        "started_unix_ms": 42,
        "entries": [
            { "file": "connections.toml", "backup": "connections.toml", "created": false }
        ]
    });
    fs::write(
        user_dir.join(".setup-commit.json"),
        serde_json::to_vec(&marker).unwrap(),
    )
    .unwrap();
    user_dir
}

#[test]
fn setup_noninteractive_never_prompts() {
    let root = isolated_root("noninteractive");
    // Both spellings refuse: the explicit flag, and stdin that cannot be a
    // terminal (/dev/null). Neither may read stdin or write a file — a flow
    // that prompted would have hit EOF and cancelled with exit 0 instead.
    for args in [vec!["setup", "--non-interactive"], vec!["setup"]] {
        let stdin = fs::File::open("/dev/null").unwrap();
        let out = saya_command(&root)
            .args(&args)
            .stdin(Stdio::from(stdin))
            .output()
            .unwrap();
        let stderr = stderr_of(&out);
        assert_eq!(out.status.code(), Some(2), "exit 2 for {args:?}: {stderr}");
        assert!(
            stderr.contains("saya setup is interactive"),
            "the guidance names the interactive requirement: {stderr}"
        );
        assert!(
            stderr.contains("saya config init") && stderr.contains("saya demo"),
            "the guidance offers the scripted alternatives: {stderr}"
        );
        assert!(
            !root.join("config-home/saya/connections.toml").exists()
                && !root.join("config-home/saya/config.toml").exists(),
            "no files were written for {args:?}"
        );
    }
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn startup_warning_printed_when_a_marker_exists() {
    let root = isolated_root("warning");
    let user_dir = stage_marker(&root);
    let out = saya_command(&root)
        .args(["--non-interactive", "config", "show"])
        .output()
        .unwrap();
    let stderr = stderr_of(&out);
    assert_eq!(
        out.status.code(),
        Some(0),
        "the command still succeeds: {stderr}"
    );
    assert!(
        stderr.contains("An interrupted `saya setup` left a recovery marker"),
        "the warning names the interruption: {stderr}"
    );
    assert!(
        stderr.contains(user_dir.to_str().unwrap()),
        "the warning names the directory: {stderr}"
    );
    assert!(
        user_dir.join(".setup-commit.json").exists(),
        "never auto-restores"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn startup_marker_read_error_is_a_warning_not_a_crash() {
    let root = isolated_root("warning-corrupt");
    let user_dir = root.join("config-home").join("saya");
    fs::create_dir_all(&user_dir).unwrap();
    fs::write(user_dir.join(".setup-commit.json"), b"not json").unwrap();
    let out = saya_command(&root)
        .args(["--non-interactive", "config", "show"])
        .output()
        .unwrap();
    let stderr = stderr_of(&out);
    assert_eq!(out.status.code(), Some(0), "no crash: {stderr}");
    assert!(
        stderr.contains("could not check for an interrupted"),
        "the read error is a single warning: {stderr}"
    );
    assert!(
        !stderr.contains("recovery marker in"),
        "no false pending claim: {stderr}"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn completions_stay_silent_when_a_marker_exists() {
    let root = isolated_root("warning-completions");
    stage_marker(&root);
    let out = saya_command(&root)
        .args(["completions", "--shell", "bash"])
        .output()
        .unwrap();
    let stderr = stderr_of(&out);
    assert_eq!(out.status.code(), Some(0), "{stderr}");
    assert!(
        !stderr.contains("recovery marker"),
        "completions never carry the setup warning: {stderr}"
    );
    let _ = fs::remove_dir_all(&root);
}
#[path = "setup_cli/guided_flow.rs"]
mod guided_flow;
