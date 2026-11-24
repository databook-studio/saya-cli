use super::support::{Q04_COMMANDS, env, exit_zero, slash_turn};

/// `/permissions` answers in the line REPL too — the headless loop prints the
/// one view: exactly the five facts, and `/grants` as its alias with the
/// once-per-session note.
#[test]
fn permissions_answers_with_the_five_facts_in_the_line_repl() {
    let dir = env("permissions");
    let output = slash_turn(&dir, "permissions.txt", "/permissions");
    exit_zero(&output, "the /permissions turn");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    for label in [
        "Mode:",
        "Grants:",
        "Deny list:",
        "Host lane:",
        "Data sharing:",
    ] {
        assert!(
            stdout.contains(label),
            "the headless view states {label}: {stdout}"
        );
    }
    assert!(
        !stdout.contains("note:"),
        "the canonical spelling states no alias note: {stdout}"
    );
    let alias = slash_turn(&dir, "grants.txt", "/grants");
    exit_zero(&alias, "the /grants turn");
    let alias_stdout = String::from_utf8_lossy(&alias.stdout).into_owned();
    assert!(
        alias_stdout.contains("note: /grants is now /permissions"),
        "the alias states its note: {alias_stdout}"
    );
    for label in [
        "Mode:",
        "Grants:",
        "Deny list:",
        "Host lane:",
        "Data sharing:",
    ] {
        assert!(
            alias_stdout.contains(label),
            "the alias answers the one view, {label} included: {alias_stdout}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir.root);
}

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
