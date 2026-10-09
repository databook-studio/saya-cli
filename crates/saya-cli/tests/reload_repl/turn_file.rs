use std::process::Command;

use super::support::{isolated, scratch_root, user_config};

/// `--turn-file /reload`: one verbatim turn, the plan printed, exit 0.
#[test]
fn turn_file_reload_prints_the_plan_and_exits_zero() {
    let root = scratch_root("turn-file");
    std::fs::create_dir_all(user_config(&root).parent().unwrap()).unwrap();
    std::fs::write(
        user_config(&root),
        "[ai]\nmodel = \"m1\"\n[run]\nmax_rows = 100\n",
    )
    .unwrap();
    let turn = root.join("turn.txt");
    std::fs::write(&turn, "/reload").unwrap();

    let workspace = root.join("ws");
    std::fs::create_dir_all(&workspace).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_saya"));
    command
        .arg("--workspace")
        .arg(&workspace)
        .arg("--approval-mode")
        .arg("read-only")
        .arg("--turn-file")
        .arg(&turn);
    isolated(&mut command, &root);
    let output = command.output().unwrap();

    assert_eq!(output.status.code(), Some(0), "exit 0: {:?}", output.status);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Configuration is unchanged."),
        "the plan is printed: {stdout}"
    );
    let _ = std::fs::remove_dir_all(root);
}
