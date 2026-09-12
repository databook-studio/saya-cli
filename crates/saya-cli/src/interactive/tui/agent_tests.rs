//! Approval parity over the four decider construction sites: the TUI's
//! [`ChannelApproval`], the terminal decider (`TerminalApproval` with
//! prompting), the run's decider (`commands/run/assembly.rs`), and the
//! headless fallback (`agent/runtime.rs`). The run's decider and the headless
//! fallback are the same `TerminalApproval::new(mode, false)` construction —
//! one instance covers both sites. Whatever the engine's `SessionPolicy`
//! decides, every path must decide; no frontend carries its own mode match.
//!
//! The one cell no test may drive is the terminal's `ask` with prompting
//! allowed — it would read the real stdin. Its prompt sentence is pinned by
//! `prompt_approval_tests::ask_prompt_keeps_the_sql_sentence_for_sql_tools`,
//! and the TUI's answered ask is driven in full below.

use super::{ChannelApproval, StreamMsg};
use crate::agent::tools::DatabaseTools;
use crate::prompt_approval::TerminalApproval;
use saya_agent::{
    ApprovalDecider, ApprovalDecision, ApprovalPolicy, LocalStateEffect, SessionPolicy,
    ToolDefinition, ToolEffect,
};
use tokio::sync::mpsc::unbounded_channel;

fn read_shaped_tool() -> ToolDefinition {
    let tools = DatabaseTools::definitions(true, false, false, false);
    tools
        .iter()
        .find(|tool| tool.name == "bounded_sql_query")
        .expect("bounded_sql_query is defined")
        .clone()
}

fn side_effecting_tool() -> ToolDefinition {
    ToolDefinition {
        name: "run_program".into(),
        description: "spawns a process outside the agent".into(),
        read_only: false,
        parameters: serde_json::json!({"type": "object"}),
        effect: ToolEffect {
            database_data: false,
            external_side_effect: true,
            requires_approval: true,
            local_state: LocalStateEffect::None,
        },
        completion: None,
    }
}

#[tokio::test]
async fn the_four_approval_paths_decide_what_the_engine_decides() {
    for tool in [read_shaped_tool(), side_effecting_tool()] {
        for mode in [
            ApprovalPolicy::ReadOnly,
            ApprovalPolicy::Never,
            ApprovalPolicy::Ask,
        ] {
            let expected =
                SessionPolicy::new(mode).resolve(&tool.effect, None) == ApprovalDecision::Allow;
            let what = format!("{} under {mode:?}", tool.name);
            // 1. The TUI decider, the construction `start` uses, with nobody
            // to answer: the channel is closed, so an ask falls back to deny.
            let (tx, rx) = unbounded_channel();
            drop(rx);
            let tui = ChannelApproval::new(tx, mode);
            assert_eq!(
                tui.approve(&tool, &serde_json::json!({"sql": "SELECT 1"}))
                    .await,
                expected,
                "TUI decider: {what}"
            );
            // 2. The terminal decider with prompting allowed: read-only and
            // never decide without reaching the prompt, so their cells are
            // drivable; ask is the stdin cell the header excludes.
            if mode != ApprovalPolicy::Ask {
                let terminal = TerminalApproval::new(mode, true);
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
            let headless = TerminalApproval::new(mode, false);
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

/// The TUI's answered ask: a yes allows, a no denies — the modal round trip
/// the closed-channel cells above cannot cover.
#[tokio::test]
async fn the_tuis_answered_ask_allows_a_yes_and_denies_a_no() {
    let tool = read_shaped_tool();
    for (answer, expected) in [(true, true), (false, false)] {
        let (tx, mut rx) = unbounded_channel();
        let tui = ChannelApproval::new(tx.clone(), ApprovalPolicy::Ask);
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
            if answer { "yes" } else { "no" },
            if expected { "allow" } else { "deny" }
        );
        answerer.await.expect("the answerer completes");
    }
}
