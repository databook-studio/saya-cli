use std::{
    fs,
    path::Path,
    path::PathBuf,
    process::{Command as ProcessCommand, Output},
};

pub(crate) fn test_root(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("saya-cli-{label}-{}", std::process::id()));
    if root.exists() {
        let _ = fs::remove_dir_all(&root);
    }
    fs::create_dir_all(&root).unwrap();
    root
}

pub(crate) fn saya_process(root: &Path, args: &[&str]) -> Output {
    ProcessCommand::new(env!("CARGO_BIN_EXE_saya"))
        .args(args)
        .current_dir(root)
        .env("SAYA_CONFIG_HOME", root.join("user-config"))
        .output()
        .unwrap()
}

pub(crate) fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

pub(crate) fn run_cli(global: &[&str], command: &[&str], state: &Path) -> Output {
    std::process::Command::new(env!("CARGO_BIN_EXE_saya"))
        .args(global)
        .args(command)
        .env("SAYA_STATE_DB", state)
        .output()
        .unwrap()
}
