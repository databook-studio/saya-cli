//! The launchers a denied program still arrived behind (SEC3). The deny list
//! binds the `shell` tool through the one matcher, and the matcher resolves the
//! program a command really runs: through a command-running prefix (`timeout`,
//! `nice`, `nohup`, `stdbuf`, `time`, `xargs`, `find`'s `-exec`), and into an
//! `eval` script — and where an interpreter's inline code cannot be read, it
//! refuses a command that names a denied program rather than running it.

use saya_agent::ApprovalPolicy;
use serde_json::json;

use super::deny_shell_tests::{approving_prompt, launched, shell};

/// Every launcher form of a denied `curl`, one spelling each.
const LAUNCHER_FORMS: &[&str] = &[
    "xargs curl",
    "xargs -I{} curl {}",
    "find . -exec curl {} \\;",
    "find . -execdir curl {} +",
    "eval \"curl x\"",
    "eval curl x",
    "timeout 5 curl x",
    "timeout -s KILL 5 curl x",
    "nice -n 5 curl x",
    "nice curl x",
    "nohup curl x",
    "stdbuf -oL curl x",
    "stdbuf -o L curl x",
    "time curl x",
    "python3 -c \"import os; os.system('curl x')\"",
    "python -c \"import os; os.system('curl x')\"",
    "perl -e 'system(\"curl x\")'",
    "ruby -e 'system(\"curl x\")'",
    "node -e 'require(\"child_process\").execSync(\"curl x\")'",
    "awk 'BEGIN{system(\"curl x\")}'",
    "gawk 'BEGIN{system(\"curl x\")}'",
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

/// One form still runs under `bypass`: no refusal, and the call is approved.
async fn runs(command: &str) {
    let rules = launched(&["curl"]);
    let policy = shell(ApprovalPolicy::Bypass, None, &rules);
    let call = json!({ "command": command });
    let refused = policy.refusal_detail(&call);
    assert!(
        refused.is_none(),
        "`{command}` names no denied program, but was refused: {refused:?}"
    );
    assert!(
        policy.approve(&call).await,
        "`{command}` still runs under bypass"
    );
}

/// 1. Every launcher form of a denied `curl` is refused under `bypass`, `ask`
///    and `accept-edits`. `bypass` is the mode with no question left to ask,
///    and the two asking modes must not reroute or approve the same command.
#[tokio::test]
async fn every_launcher_form_is_refused_in_every_mode() {
    for mode in [
        ApprovalPolicy::Bypass,
        ApprovalPolicy::Ask,
        ApprovalPolicy::AcceptEdits,
    ] {
        for command in LAUNCHER_FORMS {
            refuses(mode, command).await;
        }
    }
}

/// 2. The launchers compose: a shell script or an `eval` script behind one is
///    read the same way, and an interpreter behind one is still unreadable code.
#[tokio::test]
async fn a_script_behind_a_launcher_is_refused() {
    for mode in [
        ApprovalPolicy::Bypass,
        ApprovalPolicy::Ask,
        ApprovalPolicy::AcceptEdits,
    ] {
        refuses(mode, "xargs -I{} sh -c 'curl x'").await;
        refuses(mode, "timeout 5 bash -c \"curl x\"").await;
        refuses(mode, "eval \"sh -c 'curl x'\"").await;
        refuses(
            mode,
            "timeout 5 python3 -c \"import os; os.system('curl x')\"",
        )
        .await;
        refuses(mode, "find . -exec perl -e 'system(\"curl x\")' {} \\;").await;
    }
}

/// 3. The earlier spellings still hold: SEC1's prefixes and paths, SEC2's
///    environment, basenames, and shell scripts, and the fail-closed depth
///    bound.
#[tokio::test]
async fn the_earlier_spellings_are_still_refused() {
    for mode in [
        ApprovalPolicy::Bypass,
        ApprovalPolicy::Ask,
        ApprovalPolicy::AcceptEdits,
    ] {
        for command in [
            "curl x",
            "/usr/bin/curl x",
            "env -i curl x",
            "FOO=1 curl x",
            "sh -c 'curl x'",
            "bash -c \"curl x\"",
        ] {
            refuses(mode, command).await;
        }
        refuses(mode, &nested(3)).await;
        refuses(mode, &nested(4)).await;
    }
}

/// 4. No over-blocking: an unrelated use of the same launchers still runs, and
///    `curling` is never the denied `curl` under any launcher.
#[tokio::test]
async fn an_unrelated_use_still_runs_and_curling_is_never_refused() {
    for command in [
        "xargs echo",
        "xargs -I{} echo {}",
        "timeout 5 ls",
        "nice -n 5 ls",
        "nohup ls",
        "stdbuf -oL ls",
        "time ls",
        "find . -name x",
        "find . -name curl",
        "find . -exec echo {} \\;",
        "eval echo hi",
        "sh -c 'xargs echo'",
        "python3 -c \"print(1)\"",
        "awk '{print $1}'",
    ] {
        runs(command).await;
    }
    for command in [
        "curling",
        "curling x",
        "xargs curling",
        "xargs -I{} curling {}",
        "find . -exec curling {} \\;",
        "eval curling x",
        "timeout 5 curling x",
        "nice -n 5 curling x",
        "nohup curling x",
        "stdbuf -oL curling x",
        "time curling x",
        "timeout 5 sh -c 'curling x'",
        "python3 -c \"print('curling')\"",
    ] {
        runs(command).await;
    }
}
