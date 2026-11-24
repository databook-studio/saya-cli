//! The deny side's reading of a segment: the leading shapes a program can
//! hide behind are stripped before the rule is compared, and nothing else is.
//!
//! Split from `rules_tests` because that file is already over the reviewed
//! length; the behaviour is the deny half of the one matcher.

use super::{CommandDecision, CommandRule, CommandRules, decide};

fn rules(deny: &[&str]) -> CommandRules {
    CommandRules {
        allow: Vec::new(),
        deny: deny
            .iter()
            .map(|rule| CommandRule::parse(rule).expect("a valid test rule"))
            .collect(),
        diagnostics: Vec::new(),
        ..Default::default()
    }
}

fn denied<'a>(rules: &'a CommandRules, command: &str) -> Option<&'a str> {
    match decide(rules, command, None) {
        CommandDecision::Deny(rule) => Some(rule.text()),
        _ => None,
    }
}

/// A bare `env`, a leading assignment, an absolute path, a subshell's
/// parentheses and a substitution's openers all name the same program, so the
/// deny side reads through every one of them.
#[test]
fn the_deny_side_reads_through_the_leading_wrappers() {
    let deny_rm = rules(&["rm"]);
    for command in [
        "rm -rf x",
        "/bin/rm -rf x",
        "env rm -rf x",
        "FOO=1 rm -rf x",
        "(rm -rf x)",
        "$(rm -rf x)",
        "`rm -rf x`",
        "( rm -rf x )",
        "$( rm -rf x )",
        "` rm -rf x `",
        "echo hi && env FOO=1 rm -rf x",
    ] {
        assert_eq!(
            denied(&deny_rm, command),
            Some("rm"),
            "must deny: {command}"
        );
    }
}

/// Resolving the program is not a prefix match on the text: a program that
/// merely begins with the denied name, and a name stated as an argument, are
/// still not the program — under every wrapper the deny side reads through.
#[test]
fn the_deny_side_does_not_read_a_near_name_as_the_program() {
    let deny_rm = rules(&["rm"]);
    for command in [
        "rmx -rf x",
        "echo rm",
        "command rmx -rf x",
        "./rmx -rf x",
        "env -i rmx -rf x",
        "sh -c 'rmx -rf x'",
    ] {
        assert_eq!(denied(&deny_rm, command), None, "must not deny: {command}");
    }
}

/// `nesting` levels of `sh -c` around `rm -rf x`: 1 is `sh -c "rm -rf x"`.
fn nested(nesting: usize) -> String {
    let mut command = "rm -rf x".to_owned();
    for _ in 0..nesting {
        command = format!("sh -c \"{command}\"");
    }
    command
}

/// The deny side resolves the program a command really runs: a `command`,
/// `builtin` or `exec` prefix, an `env` with any options, a path's basename,
/// and a shell's `-c` script read with the same matcher.
#[test]
fn the_deny_side_resolves_the_program_a_command_really_runs() {
    let deny_rm = rules(&["rm"]);
    for command in [
        "command rm -rf x",
        "builtin rm -rf x",
        "exec rm -rf x",
        "./rm -rf x",
        "bin/rm -rf x",
        "../bin/rm -rf x",
        "env -i rm -rf x",
        "env -u HOME rm -rf x",
        "env -i FOO=1 rm -rf x",
        "sh -c 'rm -rf x'",
        "bash -c \"rm -rf x\"",
        "zsh -lc 'rm -rf x'",
        "dash -c 'rm -rf x'",
        "sh -c \"bash -c 'rm -rf x'\"",
    ] {
        assert_eq!(
            denied(&deny_rm, command),
            Some("rm"),
            "must deny: {command}"
        );
    }
    // Three levels of script are still read.
    assert_eq!(denied(&deny_rm, &nested(3)), Some("rm"));
}

/// Past the recursion bound, or where a script cannot be read, the deny side
/// cannot decide, so it fails closed: a command that names a denied program
/// anywhere is refused.
#[test]
fn a_command_the_deny_side_cannot_read_is_refused_not_allowed() {
    let deny_rm = rules(&["rm"]);
    assert_eq!(denied(&deny_rm, &nested(4)), Some("rm"));
    assert_eq!(denied(&deny_rm, &nested(9)), Some("rm"));
    // A script whose quotes do not balance is unreadable to a shell too, so the
    // denied name it mentions is refused rather than allowed.
    assert_eq!(denied(&deny_rm, "sh -c \"echo don't rm x\""), Some("rm"));
    // Failing closed is not a licence to refuse a name it never read: a
    // deeper script naming no denied program is still not a denial.
    let elsewhere = "echo hi";
    let mut deeper = elsewhere.to_owned();
    for _ in 0..4 {
        deeper = format!("sh -c \"{deeper}\"");
    }
    assert_eq!(denied(&deny_rm, &deeper), None);
}

/// `env` with an option that takes its own argument still names the program it
/// runs, and `env` alone is still a program in its own right: `--deny env` and
/// `--deny rm` bind their own spelling, not each other's.
#[test]
fn a_deny_rule_naming_env_still_binds_env_alone() {
    assert_eq!(denied(&rules(&["env"]), "env"), Some("env"));
    assert_eq!(denied(&rules(&["env"]), "env FOO=1 rm -rf x"), None);
    assert_eq!(denied(&rules(&["env"]), "env -i rm -rf x"), None);
    assert_eq!(denied(&rules(&["rm"]), "env FOO=1 rm -rf x"), Some("rm"));
    assert_eq!(denied(&rules(&["rm"]), "env -u HOME rm -rf x"), Some("rm"));
}
