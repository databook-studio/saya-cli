use super::support::{env, exit_zero, saya};

/// The 1 KiB preference text bound, by behaviour: 1024 bytes are admitted,
/// 1025 are refused with the usage exit code. (The 64-item cap is already
/// covered by `saya-types/tests/project_preference.rs`.)
#[test]
fn preferences_admit_1024_bytes_and_refuse_1025() {
    let env = env("preference-bound");
    let run = |args: &[&str]| {
        let mut command = saya(&env);
        command.args(args);
        command.output().unwrap()
    };
    let at_limit = "x".repeat(1024);
    let over_limit = "x".repeat(1025);
    let output = run(&["preferences", "add", &at_limit]);
    exit_zero(&output, "preferences add at exactly 1024 bytes");
    let output = run(&["preferences", "add", &over_limit]);
    assert_eq!(
        output.status.code(),
        Some(2),
        "a 1025-byte preference is refused with exit 2; stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let _ = std::fs::remove_dir_all(&env.root);
}
