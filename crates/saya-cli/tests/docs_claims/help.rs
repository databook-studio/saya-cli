use super::support::{Q04_COMMANDS, env, exit_zero, slash_turn};

/// Every command the docs document must exist where a reader will look for
/// it: the binary's own `/help` listing.
#[test]
fn every_documented_command_appears_in_the_real_help_output() {
    for line in Q04_COMMANDS {
        let env = env("help");
        let output = slash_turn(&env, "help.txt", "/help");
        exit_zero(&output, "the /help turn");
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        assert!(
            stdout.contains(line),
            "the real /help output must name {line}: {stdout}"
        );
        let _ = std::fs::remove_dir_all(&env.root);
    }
}
