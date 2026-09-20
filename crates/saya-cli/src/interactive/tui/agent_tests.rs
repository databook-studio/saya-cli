//! Approval parity over the four decider construction sites: the TUI's
//! [`ChannelApproval`], the terminal decider (`TerminalApproval` with
//! prompting), the run's decider (`commands/run/assembly.rs`), and the
//! headless fallback (`agent/runtime.rs`). The run's decider and the headless
//! fallback are the same `TerminalApproval::new(mode, false)` construction —
//! one instance covers both sites. Whatever the engine's `SessionPolicy`
//! decides, every path must decide; no frontend carries its own mode match.
//!
//! The one cell no test may drive is the terminal's `ask` with prompting
//! allowed — it would read the real stdin.
//!
//! The session's policy is hoisted to session lifetime: each turn's decider
//! is built over a clone of the one session policy, and a clone shares the
//! grant store — the tests below pin that a grant made in one turn is in
//! force in the next, and that it never answers a different shape.
//!
//! The prompt sentence is pinned by `approval_facts_tests` (one snapshot per
//! tool family); the TUI's answered ask is driven in full below.

#[cfg(test)]
#[path = "agent_deny_journal.rs"]
mod deny_journal;
#[cfg(test)]
#[path = "agent_deny_words.rs"]
mod deny_words;
#[cfg(test)]
#[path = "agent_grants.rs"]
mod grants;
#[cfg(test)]
#[path = "agent_support.rs"]
mod support;

use super::{ChannelApproval, StreamMsg};
use crate::approval_facts::ApprovalFacts;
use crate::grant_token::TurnPrimary;
use crate::prompt_approval::TerminalApproval;
use saya_agent::{
    ApprovalChoice, ApprovalDecider, ApprovalDecision, ApprovalPolicy, SessionPolicy,
};
use tokio::sync::mpsc::unbounded_channel;

use support::{read_shaped_tool, side_effecting_tool};

#[tokio::test]
async fn the_four_approval_paths_decide_what_the_engine_decides() {
    for tool in [read_shaped_tool(), side_effecting_tool()] {
        for mode in [
            ApprovalPolicy::ReadOnly,
            ApprovalPolicy::Never,
            ApprovalPolicy::Ask,
            ApprovalPolicy::Bypass,
        ] {
            let expected =
                SessionPolicy::new(mode).resolve(&tool.effect, None) == ApprovalDecision::Allow;
            let what = format!("{} under {mode:?}", tool.name);
            // 1. The TUI decider, the construction `start` uses, with nobody
            // to answer: the channel is closed, so an ask falls back to deny.
            // Under bypass the engine never asks, so the closed channel is
            // not consulted — the mode allows.
            let (tx, rx) = unbounded_channel();
            drop(rx);
            let tui = ChannelApproval::new(
                tx,
                SessionPolicy::new(mode),
                TurnPrimary::default(),
                crate::approval_facts::ApprovalFacts::default(),
                None,
            );
            assert_eq!(
                tui.approve(&tool, &serde_json::json!({"sql": "SELECT 1"}))
                    .await,
                expected,
                "TUI decider: {what}"
            );
            // 2. The terminal decider with prompting allowed: read-only and
            // never decide without reaching the prompt, so their cells are
            // drivable; ask is the stdin cell the header excludes. Bypass
            // resolves Allow, so it too never reaches the prompt.
            if mode != ApprovalPolicy::Ask {
                let terminal = TerminalApproval::new(
                    mode,
                    true,
                    TurnPrimary::default(),
                    ApprovalFacts::default(),
                );
                assert_eq!(
                    terminal
                        .approve(&tool, &serde_json::json!({"sql": "SELECT 1"}))
                        .await,
                    expected,
                    "terminal decider: {what}"
                );
            }
            // 3 + 4. The run's decider and the headless fallback are the same
            // `TerminalApproval::new(mode, false)` construction.
            let headless = TerminalApproval::new(
                mode,
                false,
                TurnPrimary::default(),
                ApprovalFacts::default(),
            );
            assert_eq!(
                headless
                    .approve(&tool, &serde_json::json!({"sql": "SELECT 1"}))
                    .await,
                expected,
                "run's and headless decider: {what}"
            );
        }
    }
}

/// Under bypass all four decider paths allow exactly what the engine
/// allows — everything — without consulting a channel, a prompt, or a grant.
/// The parity loop above covers every mode's cell; this one pins the bypass
/// rows by name, because the mode that claims "everything runs" is the one
/// where a frontend drifting into its own refusal would be least visible.
#[tokio::test]
async fn all_four_decider_paths_allow_what_the_engine_allows_under_bypass() {
    for tool in [read_shaped_tool(), side_effecting_tool()] {
        let expected = SessionPolicy::new(ApprovalPolicy::Bypass).resolve(&tool.effect, None)
            == ApprovalDecision::Allow;
        assert!(
            expected,
            "the engine allows {} under bypass, whatever its shape",
            tool.name
        );
        let what = format!("{} under bypass", tool.name);
        // 1. The TUI decider: closed channel, nobody to answer — and no ask
        // exists to answer.
        let (tx, rx) = unbounded_channel();
        drop(rx);
        let tui = ChannelApproval::new(
            tx,
            SessionPolicy::new(ApprovalPolicy::Bypass),
            TurnPrimary::default(),
            crate::approval_facts::ApprovalFacts::default(),
            None,
        );
        assert!(
            tui.approve(&tool, &serde_json::json!({"sql": "SELECT 1"}))
                .await,
            "TUI decider: {what}"
        );
        // 2. The terminal decider with a prompt surface: bypass reaches no
        // prompt — the engine answers before the surface would.
        let terminal = TerminalApproval::new(
            ApprovalPolicy::Bypass,
            true,
            TurnPrimary::default(),
            ApprovalFacts::default(),
        );
        assert!(
            terminal
                .approve(&tool, &serde_json::json!({"sql": "SELECT 1"}))
                .await,
            "terminal decider: {what}"
        );
        // 3 + 4. The run's decider and the headless fallback: the same
        // `TerminalApproval::new(mode, false)` construction.
        let headless = TerminalApproval::new(
            ApprovalPolicy::Bypass,
            false,
            TurnPrimary::default(),
            ApprovalFacts::default(),
        );
        assert!(
            headless
                .approve(&tool, &serde_json::json!({"sql": "SELECT 1"}))
                .await,
            "run's and headless decider: {what}"
        );
    }
}

/// The TUI's answered ask: allow-once allows, a deny denies — the modal round
/// trip the closed-channel cells above cannot cover.
#[tokio::test]
async fn the_tuis_answered_ask_allows_allow_once_and_denies_a_deny() {
    let tool = read_shaped_tool();
    for (answer, expected) in [
        (ApprovalChoice::AllowOnce, true),
        (ApprovalChoice::Deny, false),
    ] {
        let (tx, mut rx) = unbounded_channel();
        let tui = ChannelApproval::new(
            tx.clone(),
            SessionPolicy::new(ApprovalPolicy::Ask),
            TurnPrimary::default(),
            crate::approval_facts::ApprovalFacts::default(),
            None,
        );
        let answerer = tokio::spawn(async move {
            if let Some(StreamMsg::ApprovalRequest { respond, .. }) = rx.recv().await {
                let _ = respond.send(answer);
            }
        });
        assert_eq!(
            tui.approve(&tool, &serde_json::json!({"sql": "SELECT 1"}))
                .await,
            expected,
            "the TUI's {} must {} the ask",
            if expected { "allow once" } else { "deny" },
            if expected { "allow" } else { "deny" }
        );
        answerer.await.expect("the answerer completes");
    }
}
