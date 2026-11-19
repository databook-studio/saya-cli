//! The `shell` grant key: the job's own command tool suggests an exact
//! command prefix, only where the command is one plain segment, and never in
//! the same words as `run_command`'s own family.

use super::host_facts;
use crate::approval_facts::ApprovalFacts;
use crate::grant_token::grant_token;
use proptest::prelude::*;
use proptest::test_runner::TestRunner;
use saya_agent::{ApprovalDecision, ApprovalPolicy, SessionPolicy};
use serde_json::json;

/// The suggester's answer for one `shell` command on the composed route.
fn suggested(command: &str) -> Option<String> {
    grant_token("shell", &json!({ "command": command }), None, &host_facts())
}

/// A plain command names its first two words — and a one-word command names
/// the one it has.
#[test]
fn a_plain_command_suggests_its_first_two_words() {
    assert_eq!(
        suggested("cargo test -p saya-cli").as_deref(),
        Some("shell:cargo test"),
        "the job shell's key is the command's first two words"
    );
    assert_eq!(
        suggested("ls").as_deref(),
        Some("shell:ls"),
        "a one-word command names the one word it has"
    );
    assert_eq!(
        suggested("cargo test --release").as_deref(),
        Some("shell:cargo test"),
        "the key is the prefix, not the whole command"
    );
    assert_eq!(
        suggested("  cargo   test  ").as_deref(),
        Some("shell:cargo test"),
        "whitespace separates words and never joins them"
    );
}

/// Everything the key refuses: a compound command, a redirect, a
/// substitution, a heredoc, unbalanced quotes, a leading assignment or `cd`,
/// and a command with no readable words at all. Each one keeps asking.
#[test]
fn a_command_that_is_not_one_plain_segment_suggests_nothing() {
    assert_eq!(
        suggested("cargo test"),
        Some("shell:cargo test".to_owned()),
        "the positive control: the plain shape does suggest a key"
    );
    for command in [
        "cargo test && rm -rf x",
        "cargo test || true",
        "cargo test | tee o",
        "cargo test |& tee o",
        "cargo test; ls",
        "cargo test &",
        "cargo test > out.txt",
        "cargo test >> out.txt",
        "cargo test > /dev/null",
        "echo $(id)",
        "echo `id`",
        "cat <(id)",
        "cat <<EOF",
        "cat <<'EOF'\nbody\nEOF",
        "echo \"a & b\"",
        "cargo test --filter 'a|b'",
        "cargo test --filter \"a;b\"",
        "cargo test 2>&1",
        "cargo test &> /dev/null",
        "cargo test\ncargo build",
        "cargo test\r\ncargo build",
        "echo \"unbalanced",
        "echo 'unbalanced",
        "FOO=1 cargo test",
        "cd /tmp && ls",
        "cd /tmp",
        "",
        "   ",
    ] {
        assert_eq!(
            suggested(command),
            None,
            "`{command}` is not one plain segment, so it keeps asking"
        );
    }
}

/// No route, no command, no token: the same refusal shape every family's
/// suggester has.
#[test]
fn no_key_without_the_host_route_or_a_readable_command() {
    assert_eq!(
        grant_token(
            "shell",
            &json!({ "command": "cargo test" }),
            None,
            &ApprovalFacts::default()
        ),
        None,
        "a shell token names the host route, so it is offered only where one composed"
    );
    assert_eq!(
        suggested("cargo test"),
        Some("shell:cargo test".to_owned()),
        "the composed route offers the key (the other cases here are the refusal)"
    );
    assert_eq!(
        grant_token("shell", &json!({}), None, &host_facts()),
        None,
        "a call with no `command` names nothing to grant"
    );
    assert_eq!(
        grant_token("shell", &json!({ "command": 7 }), None, &host_facts()),
        None,
        "a non-string command names nothing to grant"
    );
}

/// Family isolation: `shell:` and `command:` are two words, and neither
/// pre-answers the other's door.
#[test]
fn a_shell_key_and_a_command_key_never_stand_in_for_each_other() {
    let shell = crate::agent::tools::shell_policy::definition();
    let command = json!({ "command": "cargo test" });
    let policy = SessionPolicy::new(ApprovalPolicy::Ask);
    policy.grants().grant("command:cargo");
    assert_eq!(
        grant_token(&shell.name, &command, None, &host_facts()).as_deref(),
        Some("shell:cargo test"),
        "the shell call's own key"
    );
    assert_eq!(
        policy.resolve(&shell.effect, Some("shell:cargo test")),
        ApprovalDecision::Ask,
        "a `command:cargo` grant admits no shell call"
    );

    let run = crate::interactive::session_definitions::run_command();
    let argv = json!({ "program": "cargo", "args": ["test"] });
    let policy = SessionPolicy::new(ApprovalPolicy::Ask);
    policy.grants().grant("shell:cargo test");
    assert_eq!(
        grant_token(&run.name, &argv, None, &host_facts()).as_deref(),
        Some("command:cargo"),
        "the run-command call's own key"
    );
    assert_eq!(
        policy.resolve(&run.effect, Some("command:cargo")),
        ApprovalDecision::Ask,
        "a `shell:cargo test` grant admits no run_command call"
    );
}

/// The refused characters: any key that names one of these would claim a
/// prefix the operator never read.
const REFUSED: &[&str] = &["&", "|", ";", ">", "<", "\n", "\r", "`", "$("];

/// Arbitrary commands: raw unicode, words built from the characters a shell
/// reads as structure, and words a shell runs as themselves — so the sweep
/// reaches both the refusing and the key-making paths.
fn arbitrary_command() -> impl Strategy<Value = String> {
    const POOL: &str = "abcargo test-|&;<>`$()'\"\n\r=é☃/.";
    const PLAIN: &[&str] = &["cargo", "test", "ls", "git", "diff", "--release", "x"];
    let pool: Vec<char> = POOL.chars().collect();
    let word = proptest::collection::vec(proptest::sample::select(pool), 1..7)
        .prop_map(|chars| chars.into_iter().collect::<String>());
    let plain = proptest::sample::select(PLAIN);
    prop_oneof![
        2 => any::<String>(),
        2 => proptest::collection::vec(word, 0..5).prop_map(|words| words.join(" ")),
        3 => proptest::collection::vec(plain, 0..5).prop_map(|words| words.join(" ")),
    ]
}

/// Totality: no input panics, and every key states a command with none of the
/// refused characters in it. The sweep must reach the key-making path at least
/// once, or it would prove nothing about the keys it exists to guard.
#[test]
fn a_suggested_key_never_carries_shell_structure() {
    let keys = std::cell::Cell::new(0u32);
    let mut runner = TestRunner::new(ProptestConfig {
        cases: 2_000,
        ..ProptestConfig::default()
    });
    runner
        .run(&arbitrary_command(), |command| {
            let Some(token) = suggested(&command) else {
                return Ok(());
            };
            keys.set(keys.get() + 1);
            prop_assert!(token.starts_with("shell:"), "{token} is a shell key");
            for needle in REFUSED {
                prop_assert!(
                    !command.contains(needle),
                    "`{command}` contains {needle:?} but suggested {token}"
                );
            }
            Ok(())
        })
        .expect("every generated command holds the refusal");
    assert!(
        keys.get() > 0,
        "the sweep must exercise the key-making path, not only refusals"
    );
}
