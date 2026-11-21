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

/// Stripping is not widening: a program that merely begins with the denied
/// name, and a name stated as an argument, are still not the program.
#[test]
fn the_deny_side_does_not_read_a_near_name_as_the_program() {
    let deny_rm = rules(&["rm"]);
    assert_eq!(denied(&deny_rm, "rmx -rf x"), None);
    assert_eq!(denied(&deny_rm, "echo rm"), None);
    assert_eq!(denied(&deny_rm, "command rm -rf x"), None);
    assert_eq!(denied(&deny_rm, "./rm -rf x"), None);
}

/// `env` alone is a program, not a prefix another program hides behind: a deny
/// rule naming it still binds the bare spelling, while `env FOO=1 rm` is read
/// as the program it runs, `rm`.
#[test]
fn a_deny_rule_naming_env_still_binds_env_alone() {
    assert_eq!(denied(&rules(&["env"]), "env"), Some("env"));
    assert_eq!(denied(&rules(&["env"]), "env FOO=1 rm -rf x"), None);
    assert_eq!(denied(&rules(&["rm"]), "env FOO=1 rm -rf x"), Some("rm"));
}
