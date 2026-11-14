//! The matcher's decision table: prefix boundaries, compound commands,
//! substitution, deny precedence, and the two stripped leading shapes.

use super::{CommandDecision, CommandRule, CommandRules, decide};

fn rules(allow: &[&str], deny: &[&str]) -> CommandRules {
    CommandRules {
        allow: allow
            .iter()
            .map(|rule| CommandRule::parse(rule).expect("a valid test rule"))
            .collect(),
        deny: deny
            .iter()
            .map(|rule| CommandRule::parse(rule).expect("a valid test rule"))
            .collect(),
        diagnostics: Vec::new(),
    }
}

fn allowed(allow: &[&str], deny: &[&str], command: &str) -> bool {
    decide(&rules(allow, deny), command, None) == CommandDecision::Allow
}

fn denied<'a>(rules: &'a CommandRules, command: &str) -> Option<&'a str> {
    match decide(rules, command, None) {
        CommandDecision::Deny(rule) => Some(rule.text()),
        _ => None,
    }
}

/// A rule matches whole tokens only: the next token may follow, but a token
/// that merely starts with the rule's last word must not.
#[test]
fn a_prefix_matches_on_word_boundaries_only() {
    assert!(allowed(&["cargo test"], &[], "cargo test -p saya-cli"));
    assert!(allowed(&["cargo test"], &[], "cargo test"));
    assert!(!allowed(&["cargo test"], &[], "cargo testx"));
    assert!(!allowed(&["cargo test"], &[], "cargo"));
    assert!(!allowed(&["cargo test"], &[], "cargo  testx"));
    assert!(allowed(&["git diff"], &[], "git diff --stat HEAD"));
}

/// A compound command is allowed only when *every* segment matches.
#[test]
fn every_segment_of_a_compound_command_must_match() {
    let allow = ["cargo test", "cargo clippy"];
    for command in [
        "cargo test && cargo clippy",
        "cargo test || cargo clippy",
        "cargo test; cargo clippy",
        "cargo test | cargo clippy",
        "cargo test\ncargo clippy",
    ] {
        assert!(allowed(&allow, &[], command), "must allow: {command}");
    }
    assert!(!allowed(&["cargo test"], &[], "cargo test && rm -rf build"));
    assert!(!allowed(&["cargo test"], &[], "cargo test | tee out.txt"));
}

/// A substitution or heredoc makes the prefix claim untrue: no rule
/// auto-allows the call, whatever its first tokens are.
#[test]
fn substitution_and_heredocs_are_never_auto_allowed() {
    for command in [
        "cargo test $(date)",
        "cargo test `date`",
        "cargo test <(date)",
        "cargo test >(cat)",
        "cargo test <<EOF",
        "cargo test <<-EOF",
    ] {
        assert!(
            !allowed(&["cargo test"], &[], command),
            "must not auto-allow: {command}"
        );
    }
    // A literal `$` without a following `(` is not a substitution.
    assert!(allowed(&["echo $HOME"], &[], "echo $HOME"));
}

/// Deny wins over allow and is evaluated first.
#[test]
fn deny_wins_over_allow() {
    let rules = rules(&["cargo"], &["cargo test"]);
    assert_eq!(
        denied(&rules, "cargo test -p x"),
        Some("cargo test"),
        "the deny rule is the one named"
    );
    assert_eq!(decide(&rules, "cargo build", None), CommandDecision::Allow);
}

/// Deny still applies when the command carries a substitution: the refusal
/// does not depend on the allow side being readable.
#[test]
fn deny_applies_even_with_substitution() {
    let rules = rules(&["echo"], &["rm"]);
    assert_eq!(denied(&rules, "rm -rf / && echo $(date)"), Some("rm"));
}

/// A leading `cd <dir> &&` is stripped for matching only, and only when the
/// directory stays inside the workspace.
#[test]
fn a_leading_cd_inside_the_workspace_is_stripped() {
    let table = rules(&["cargo test"], &[]);
    let root = std::env::temp_dir().join(format!(
        "saya-rule-cd-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir_all(root.join("sub")).expect("the workspace must be creatable");
    let root = root
        .canonicalize()
        .expect("the workspace must canonicalise");
    assert_eq!(
        decide(&table, "cd sub && cargo test", Some(&root)),
        CommandDecision::Allow
    );
    let inside = root.join("sub");
    assert_eq!(
        decide(
            &table,
            &format!("cd {} && cargo test", inside.display()),
            Some(&root)
        ),
        CommandDecision::Allow
    );
    // Outside the workspace, or with traversal, the cd is not stripped and
    // the first segment matches no rule.
    assert_eq!(
        decide(&table, "cd /tmp && cargo test", Some(&root)),
        CommandDecision::Unmatched
    );
    assert_eq!(
        decide(&table, "cd ../elsewhere && cargo test", Some(&root)),
        CommandDecision::Unmatched
    );
    // Without a known workspace only a relative, traversal-free cd strips.
    assert_eq!(
        decide(&table, "cd sub && cargo test", None),
        CommandDecision::Allow
    );
    assert_eq!(
        decide(&table, "cd /tmp && cargo test", None),
        CommandDecision::Unmatched
    );
    // A directory the shell expands is not known to be inside the workspace,
    // so it is not stripped and the `cd` segment must match a rule itself.
    for command in [
        "cd ~ && cargo test",
        "cd ~/sub && cargo test",
        "cd $HOME && cargo test",
        "cd $PWD/sub && cargo test",
        "cd - && cargo test",
    ] {
        assert_eq!(
            decide(&table, command, Some(&root)),
            CommandDecision::Unmatched,
            "must not strip: {command}"
        );
    }
    assert_eq!(
        decide(
            &rules(&["cd ~", "cargo test"], &[]),
            "cd ~ && cargo test",
            Some(&root)
        ),
        CommandDecision::Allow,
        "a stated cd rule covers the segment"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// A single `&`, `|&`, a carriage return, and a trailing separator are
/// separators: the segment after them must match too, and an empty segment
/// (a trailing separator) matches nothing.
#[test]
fn background_and_carriage_return_separate_segments() {
    for command in [
        "cargo test & rm -rf ~",
        "cargo test |& rm -rf ~",
        "cargo test\r rm -rf ~",
        "cargo test &",
        "cargo test &&",
        "cargo test |",
    ] {
        assert!(
            !allowed(&["cargo test"], &[], command),
            "must not auto-allow: {command:?}"
        );
    }
    assert!(allowed(&["cargo test"], &[], "cargo test && cargo test"));
}

/// Output redirection is not part of the prefix an allow rule covers, except
/// a redirect to `/dev/null` and an fd duplication. Input redirection from a
/// file stays covered.
#[test]
fn output_redirection_is_not_covered() {
    for command in [
        "git diff > ~/.zshrc",
        "cargo test > out.txt",
        "cargo test >> out.txt",
        "cargo test &> out.txt",
        "cargo test >| out.txt",
        "cargo test 2> out.txt",
        "cargo test 2>> out.txt",
    ] {
        assert!(
            !allowed(&["git diff", "cargo test"], &[], command),
            "must not auto-allow: {command}"
        );
    }
    for command in [
        "cargo test 2>&1",
        "cargo test >&2",
        "cargo test >/dev/null 2>&1",
        "cargo test > /dev/null",
        "cargo test < input.txt",
    ] {
        assert!(
            allowed(&["cargo test"], &[], command),
            "must allow: {command}"
        );
    }
}

/// A leading environment assignment is not auto-allowed: it can change the
/// program's environment in ways the stated prefix does not cover. The deny
/// side still strips it, so a denied program cannot hide behind one.
#[test]
fn a_leading_environment_assignment_is_not_auto_allowed() {
    for command in [
        "PATH=/tmp/x cargo test",
        "RUSTC_WRAPPER=x cargo test",
        "LD_PRELOAD=x git diff",
    ] {
        assert!(
            !allowed(&["cargo test", "git diff"], &[], command),
            "must not auto-allow: {command}"
        );
    }
    // An assignment-shaped token after the program is an ordinary argument.
    assert!(allowed(&["cargo test"], &[], "cargo test FOO=bar"));
    assert!(!allowed(&["cargo test"], &[], "cargo FOO=bar test"));
    // A segment made only of assignments has no program to match.
    assert!(!allowed(&["cargo test"], &[], "FOO=bar && cargo test"));
    // Deny still sees through the assignment.
    assert_eq!(
        denied(&rules(&[], &["rm"]), "FOO=1 rm -rf x"),
        Some("rm"),
        "the deny side keeps stripping assignments"
    );
}

/// Deny matches an absolute program path by its basename, and nothing else:
/// `command rm` and `\rm` are different prefixes, as the docs say.
#[test]
fn deny_matches_the_basename_of_an_absolute_program() {
    let deny_rm = rules(&[], &["rm"]);
    assert_eq!(denied(&deny_rm, "/bin/rm -rf x"), Some("rm"));
    assert_eq!(denied(&deny_rm, "/usr/local/bin/rm x"), Some("rm"));
    assert_eq!(denied(&deny_rm, "rm -rf x"), Some("rm"));
    assert_eq!(denied(&deny_rm, "command rm -rf x"), None);
    assert_eq!(denied(&deny_rm, "\\rm -rf x"), None);
    assert_eq!(denied(&deny_rm, "./rm -rf x"), None);
    assert_eq!(
        denied(&rules(&[], &["rm -rf"]), "/bin/rm -rf x"),
        Some("rm -rf")
    );
}

/// A backslash-newline is a continuation, not a separator; an unbalanced
/// quote makes the allow side unreadable (deny is still checked).
#[test]
fn continuations_join_lines_and_unbalanced_quotes_never_allow() {
    assert!(allowed(&["cargo test"], &[], "cargo \\\ntest"));
    assert!(
        !allowed(&["cargo test"], &[], "cargo test \"unterminated"),
        "an open double quote is not a prefix claim"
    );
    assert!(
        !allowed(&["cargo test"], &[], "cargo test 'unterminated"),
        "an open single quote is not a prefix claim"
    );
    assert_eq!(
        denied(&rules(&[], &["rm"]), "rm -rf 'x"),
        Some("rm"),
        "deny does not depend on the allow side being readable"
    );
}

/// The bounds are enforced by the parser, not by a panic.
#[test]
fn rules_are_bounded_and_validated() {
    assert!(CommandRule::parse("").is_err());
    assert!(CommandRule::parse("   ").is_err());
    assert!(CommandRule::parse(&"x".repeat(super::MAX_COMMAND_RULE_BYTES + 1)).is_err());
    assert!(CommandRule::parse("cargo\ttest").is_err());
    assert!(CommandRule::parse("cargo test").is_ok());
}
