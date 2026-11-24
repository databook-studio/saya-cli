//! The deny list on the live path: `--deny <program>` bound the core `shell`,
//! and a denied command is refused before the mode, before any remembered
//! grant, and before a person is ever asked.
//!
//! Twins of `host_h3_red_tests::a_denied_name_still_refuses_under_ask_and_bypass`
//! and the bypass half of
//! `host_h3_red_tests::bypass_allows_every_host_ask_and_structural_refusals_still_refuse`,
//! which build their tool from `interactive/session_definitions::run_command`.
//! The name and the decider change (the job root's `shell`, the shell policy
//! over `ChatFront::ask`); the outcome does not: refused stays refused, and an
//! undenied command under `bypass` is admitted without a grant.

use saya_agent::ApprovalPolicy;
use serde_json::json;

use super::*;

/// A denied command is refused under every mode, including the ones whose
/// answer is otherwise automatic, and the person in front of the front is never
/// shown it. The refusal carries the shell lane's own rule wording, so the
/// model relays the policy rather than a "command not found".
#[tokio::test]
async fn the_core_shell_refuses_a_denied_command_and_never_asks_the_front() {
    let shell = core_shell();
    assert_eq!(
        shell.name, "shell",
        "the live catalogue's one command tool is `shell`"
    );
    assert!(
        shell.effect.external_side_effect && shell.effect.requires_approval,
        "the core shell reaches outside the workspace and says so: {:?}",
        shell.effect
    );
    let rules = deny_rules(&["curl"]);
    for mode in [
        ApprovalPolicy::Ask,
        ApprovalPolicy::AcceptEdits,
        ApprovalPolicy::Bypass,
    ] {
        let front = Scripted::answering(Answer::Session);
        let operator = operator(&front);
        let policy = live_shell(mode, &operator, rules.clone());
        for command in ["curl https://example.com", "echo hi && curl x"] {
            let call = json!({ "command": command });
            let reason = policy
                .refusal_detail(&call)
                .unwrap_or_else(|| panic!("{mode:?} must refuse `{command}`"));
            assert!(
                reason.contains("denied by the permission rule `curl`"),
                "{mode:?} must carry the shell lane's rule wording: {reason}"
            );
            assert!(
                !policy.approve(&call).await,
                "{mode:?} must not run `{command}`"
            );
            assert!(
                !policy.would_ask(&call),
                "{mode:?} refuses `{command}`: it never puts it to the operator"
            );
        }
        assert_eq!(
            front.asked(),
            0,
            "{mode:?} showed a denied command to a person who would have allowed it"
        );
    }
}

/// A command the list does not name is admitted under `bypass` with no grant
/// held and no question asked: the mode is the consent, and deny is the only
/// rule that outranks it.
#[tokio::test]
async fn bypass_admits_an_undenied_command_on_the_core_shell() {
    let front = Scripted::answering(Answer::Deny);
    let operator = operator(&front);
    let policy = live_shell(ApprovalPolicy::Bypass, &operator, deny_rules(&["curl"]));
    for command in ["cargo test -p saya-cli", "curly", "echo curl"] {
        let call = json!({ "command": command });
        assert!(
            policy.refusal_detail(&call).is_none(),
            "`{command}` is outside the deny list and must not be refused"
        );
        assert!(
            policy.approve(&call).await,
            "bypass admits `{command}` without a grant"
        );
    }
    assert_eq!(
        front.asked(),
        0,
        "bypass puts no question: the mode answered already"
    );
}
