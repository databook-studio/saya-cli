use super::support::{env, exit_zero, slash_turn};

/// The instruction caps are visible in real output: `/context` prints the
/// effective limits, and the numbers must equal the config constants.
#[test]
fn context_prints_the_instruction_limits_the_constants_resolve() {
    let env = env("context-limits");
    let output = slash_turn(&env, "context.txt", "/context");
    exit_zero(&output, "the /context turn");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let limits = format!(
        "project {} B, personal {} B, total {} B",
        saya_config::DEFAULT_PROJECT_INSTRUCTION_BYTES,
        saya_config::DEFAULT_PERSONAL_INSTRUCTION_BYTES,
        saya_config::DEFAULT_TOTAL_INSTRUCTION_BYTES,
    );
    assert!(
        stdout.contains(&limits),
        "the /context settings line must state {limits}; stdout: {stdout}"
    );
    let _ = std::fs::remove_dir_all(&env.root);
}
