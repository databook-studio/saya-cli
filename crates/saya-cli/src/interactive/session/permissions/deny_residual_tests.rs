//! The residual spellings a denied program still arrived in (SEC2). The deny
//! list binds the `shell` tool through the one matcher, and the matcher
//! resolves the program a command really runs: a `command`/`builtin`/`exec`
//! prefix, an `env` with any options, a path's basename, and a shell's `-c`
//! script — and it refuses a command it cannot resolve rather than running it.

use saya_agent::ApprovalPolicy;
use serde_json::json;

use super::deny_shell_tests::{approving_prompt, launched, shell};

/// Every residual form of a denied `curl`, one spelling each.
const RESIDUAL_FORMS: &[&str] = &[
    "command curl x",
    "builtin curl x",
    "exec curl x",
    "./curl x",
    "/usr/local/bin/curl x",
    "bin/curl x",
    "env -i curl x",
    "env -u HOME curl x",
    "env -i FOO=1 curl x",
    "sh -c 'curl x'",
    "bash -c \"curl x\"",
    "zsh -lc 'curl x'",
    "dash -c 'curl x'",
];

/// `nesting` levels of `sh -c` around `curl x`: 1 is `sh -c "curl x"`.
fn nested(nesting: usize) -> String {
    let mut command = "curl x".to_owned();
    for _ in 0..nesting {
        command = format!("sh -c \"{command}\"");
    }
    command
}

/// One form is refused by the rule, in this mode, with an operator present and
/// without one: a pre-answered or rerouted call would be the same evasion.
async fn refuses(mode: ApprovalPolicy, command: &str) {
    let rules = launched(&["curl"]);
    for prompt in [None, Some(approving_prompt())] {
        let policy = shell(mode, prompt, &rules);
        let call = json!({ "command": command });
        let reason = policy
            .refusal_detail(&call)
            .unwrap_or_else(|| panic!("{mode:?} must refuse `{command}` by the deny rule"));
        assert!(
            reason.contains("denied by the permission rule `curl`"),
            "{mode:?} `{command}` must carry the shell lane's rule wording: {reason}"
        );
        assert!(
            !policy.approve(&call).await,
            "{mode:?} must not approve `{command}`"
        );
        assert!(
            !policy.would_ask(&call),
            "{mode:?} refuses `{command}`: it does not put it to the operator"
        );
    }
}

/// 1. Every residual form of `curl` is refused under `bypass`, `ask` and
///    `accept-edits`. `bypass` is the mode with no question left to ask, and
///    the two asking modes must not reroute or approve the same command.
#[tokio::test]
async fn every_residual_form_is_refused_in_every_mode() {
    for mode in [
        ApprovalPolicy::Bypass,
        ApprovalPolicy::Ask,
        ApprovalPolicy::AcceptEdits,
    ] {
        for command in RESIDUAL_FORMS {
            refuses(mode, command).await;
        }
    }
}

/// 2. A shell's `-c` script is read with the same matcher, so a denied program
///    one shell deeper is still the denied program.
#[tokio::test]
async fn a_nested_shell_script_is_refused() {
    for mode in [
        ApprovalPolicy::Bypass,
        ApprovalPolicy::Ask,
        ApprovalPolicy::AcceptEdits,
    ] {
        refuses(mode, "sh -c \"bash -c 'curl x'\"").await;
    }
}

/// 3. Past the recursion bound the deny side fails closed: a command it can no
///    longer read, that names a denied program, is refused rather than allowed.
#[tokio::test]
async fn a_script_past_the_recursion_bound_is_refused() {
    for mode in [
        ApprovalPolicy::Bypass,
        ApprovalPolicy::Ask,
        ApprovalPolicy::AcceptEdits,
    ] {
        refuses(mode, &nested(4)).await;
        // Three levels are still read, not refused for depth.
        refuses(mode, &nested(3)).await;
    }
}

/// 4. Resolving the program is not widening: a different program that merely
///    starts with the denied name runs wherever it appears, under every spelling
///    the deny side reads through.
#[tokio::test]
async fn a_name_that_merely_starts_like_the_denied_one_still_runs() {
    let rules = launched(&["curl"]);
    let policy = shell(ApprovalPolicy::Bypass, None, &rules);
    for command in [
        "curling",
        "curling x",
        "command curling x",
        "./curling x",
        "env -i curling x",
        "sh -c 'curling x'",
        "echo curl",
    ] {
        let call = json!({ "command": command });
        assert!(
            policy.refusal_detail(&call).is_none(),
            "`{command}` names no denied program"
        );
        assert!(
            policy.approve(&call).await,
            "`{command}` still runs under bypass"
        );
    }
}

/// 5. An unrelated command is untouched by the widened reading.
#[tokio::test]
async fn an_unrelated_command_still_runs() {
    let rules = launched(&["curl"]);
    let policy = shell(ApprovalPolicy::Bypass, None, &rules);
    for command in ["cargo build", "echo hi", "sh -c 'echo hi'", "env -i true"] {
        let call = json!({ "command": command });
        assert!(
            policy.refusal_detail(&call).is_none(),
            "`{command}` names no denied program"
        );
        assert!(
            policy.approve(&call).await,
            "`{command}` still runs under bypass"
        );
    }
}
