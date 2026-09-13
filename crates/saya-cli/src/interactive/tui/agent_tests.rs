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
//!
//! The session's policy is hoisted to session lifetime: each turn's decider
//! is built over a clone of the one session policy, and a clone shares the
//! grant store — the tests below pin that a grant made in one turn is in
//! force in the next, and that it never answers a different shape.

use super::{ChannelApproval, StreamMsg};
use crate::agent::tools::DatabaseTools;
use crate::grant_token::TurnPrimary;
use crate::prompt_approval::TerminalApproval;
use saya_agent::{
    ApprovalChoice, ApprovalDecider, ApprovalDecision, ApprovalPolicy, LocalStateEffect,
    SessionPolicy, ToolDefinition, ToolEffect,
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
            let tui = ChannelApproval::new(tx, SessionPolicy::new(mode), TurnPrimary::default());
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
                let terminal = TerminalApproval::new(mode, true, TurnPrimary::default());
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
            let headless = TerminalApproval::new(mode, false, TurnPrimary::default());
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
        );
        assert!(
            tui.approve(&tool, &serde_json::json!({"sql": "SELECT 1"}))
                .await,
            "TUI decider: {what}"
        );
        // 2. The terminal decider with a prompt surface: bypass reaches no
        // prompt — the engine answers before the surface would.
        let terminal = TerminalApproval::new(ApprovalPolicy::Bypass, true, TurnPrimary::default());
        assert!(
            terminal
                .approve(&tool, &serde_json::json!({"sql": "SELECT 1"}))
                .await,
            "terminal decider: {what}"
        );
        // 3 + 4. The run's decider and the headless fallback: the same
        // `TerminalApproval::new(mode, false)` construction.
        let headless = TerminalApproval::new(ApprovalPolicy::Bypass, false, TurnPrimary::default());
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

/// The TUI's modal offers the SQL family's token the way the turn binds it:
/// a call naming no connection suggests `sql:<primary>` — the primary's real
/// registry name, bound into the decider by the turn — and the recorded
/// grant pre-answers the next call of the same connection.
#[tokio::test]
async fn the_tui_s_modal_offers_the_turn_s_primary_sql_token() {
    let primary = TurnPrimary::default();
    primary.bind(&crate::grant_token_tests::registry_with_primary(
        "analytics",
    ));
    let (tx, mut rx) = unbounded_channel();
    let decider =
        ChannelApproval::new(tx, SessionPolicy::new(ApprovalPolicy::Ask), primary.clone());
    let answerer = tokio::spawn(async move {
        if let Some(StreamMsg::ApprovalRequest { respond, grant, .. }) = rx.recv().await {
            assert_eq!(
                grant.as_deref(),
                Some("sql:analytics"),
                "a connectionless SQL call offers the primary's real registry name"
            );
            let _ = respond.send(ApprovalChoice::AllowSession {
                token: grant.expect("the ask offered a token"),
            });
        }
    });
    let sql = read_shaped_tool();
    assert!(
        decider
            .approve(&sql, &serde_json::json!({"sql": "SELECT 1"}))
            .await,
        "the user's session grant allows the call that asked"
    );
    answerer.await.expect("the answerer completes");
    // And an unbound handle — a decider built before its turn bound a
    // registry (or a run's construction) — offers no token at all.
    let (tx, mut rx) = unbounded_channel();
    let decider = ChannelApproval::new(
        tx,
        SessionPolicy::new(ApprovalPolicy::Ask),
        TurnPrimary::default(),
    );
    let answerer = tokio::spawn(async move {
        if let Some(StreamMsg::ApprovalRequest { grant, .. }) = rx.recv().await {
            assert_eq!(
                grant, None,
                "an unbound primary suggests no token — fail closed, never a \
                 guessed name"
            );
        }
    });
    let _ = decider
        .approve(&sql, &serde_json::json!({"sql": "SELECT 1"}))
        .await;
    answerer.await.expect("the answerer completes");
}

/// The grant outlives the turn: a session grant recorded through one turn's
/// decider is in force for a later turn's decider. Each turn builds its
/// decider over a clone of the one session policy, and a clone shares the
/// grant store — so turn four resolves the granted shape with no ask at all.
#[tokio::test]
async fn a_tui_grant_made_in_one_turn_is_in_force_in_the_next() {
    let tool = side_effecting_tool();
    let arguments = serde_json::json!({"program": "bench"});
    let policy = SessionPolicy::new(ApprovalPolicy::Ask);
    // Turn three: the ask arrives with the suggested token, and the user's
    // session grant is recorded through the decider.
    let (tx, mut rx) = unbounded_channel();
    let turn_three = ChannelApproval::new(tx, policy.clone(), TurnPrimary::default());
    let answerer = tokio::spawn(async move {
        if let Some(StreamMsg::ApprovalRequest { respond, grant, .. }) = rx.recv().await {
            assert_eq!(
                grant.as_deref(),
                Some("runner:bench"),
                "the ask must offer the token the grant would record"
            );
            let _ = respond.send(ApprovalChoice::AllowSession {
                token: grant.expect("the ask offered a token"),
            });
        }
    });
    assert!(
        turn_three.approve(&tool, &arguments).await,
        "the user's session grant allows the call that asked"
    );
    answerer.await.expect("the answerer completes");
    // Turn four: a fresh decider over the same session policy — the wiring
    // `start` uses. The same shape resolves Allow with no ask: the channel is
    // closed, so an ask would deny.
    let (tx, rx) = unbounded_channel();
    drop(rx);
    let turn_four = ChannelApproval::new(tx, policy.clone(), TurnPrimary::default());
    assert!(
        turn_four.approve(&tool, &arguments).await,
        "the grant made in turn three is still in force in turn four"
    );
    assert!(
        policy.grants().is_granted("runner:bench"),
        "the grant was recorded into the one session policy"
    );
}

/// A grant is narrow: `runner:bench` granted does not answer `runner:deploy`,
/// and `fetch:https+a.example` granted does not answer `fetch:https+b.example`
/// — a different shape still asks.
#[tokio::test]
async fn a_grant_does_not_answer_a_different_shape() {
    let policy = SessionPolicy::new(ApprovalPolicy::Ask);
    let granted = side_effecting_tool();
    let granted_args = serde_json::json!({"program": "bench"});
    let other = side_effecting_tool();
    let other_args = serde_json::json!({"program": "deploy"});

    // Turn three grants runner:bench through the ask.
    let (tx, mut rx) = unbounded_channel();
    let decider = ChannelApproval::new(tx, policy.clone(), TurnPrimary::default());
    let answerer = tokio::spawn(async move {
        if let Some(StreamMsg::ApprovalRequest { respond, grant, .. }) = rx.recv().await {
            assert_eq!(grant.as_deref(), Some("runner:bench"));
            let _ = respond.send(ApprovalChoice::AllowSession {
                token: grant.expect("the ask offered a token"),
            });
        }
    });
    assert!(decider.approve(&granted, &granted_args).await);
    answerer.await.expect("the answerer completes");

    // The granted shape is pre-answered (closed channel: an ask would deny).
    let (tx, rx) = unbounded_channel();
    drop(rx);
    let decider = ChannelApproval::new(tx, policy.clone(), TurnPrimary::default());
    assert!(
        decider.approve(&granted, &granted_args).await,
        "the granted shape runs without asking"
    );
    // A different shape still asks — and a deny there denies.
    let (tx, mut rx) = unbounded_channel();
    let decider = ChannelApproval::new(tx, policy.clone(), TurnPrimary::default());
    let answerer = tokio::spawn(async move {
        if let Some(StreamMsg::ApprovalRequest { respond, grant, .. }) = rx.recv().await {
            assert_eq!(grant.as_deref(), Some("runner:deploy"));
            let _ = respond.send(ApprovalChoice::Deny);
        }
    });
    assert!(
        !decider.approve(&other, &other_args).await,
        "`runner:bench` granted does not allow `runner:deploy`"
    );
    answerer.await.expect("the answerer completes");
    assert!(
        !policy.grants().is_granted("runner:deploy"),
        "a deny grants nothing"
    );

    // The same narrowness for fetch: one host's grant never covers another.
    let fetch = crate::interactive::session_definitions::http_fetch();
    let here = serde_json::json!({"url": "https://a.example/x"});
    let there = serde_json::json!({"url": "https://b.example/x"});
    let (tx, mut rx) = unbounded_channel();
    let decider = ChannelApproval::new(tx, policy.clone(), TurnPrimary::default());
    let answerer = tokio::spawn(async move {
        if let Some(StreamMsg::ApprovalRequest { respond, grant, .. }) = rx.recv().await {
            assert_eq!(grant.as_deref(), Some("fetch:https+a.example"));
            let _ = respond.send(ApprovalChoice::AllowSession {
                token: grant.expect("the ask offered a token"),
            });
        }
    });
    assert!(decider.approve(&fetch, &here).await, "a.example is granted");
    answerer.await.expect("the answerer completes");
    let (tx, rx) = unbounded_channel();
    drop(rx);
    let decider = ChannelApproval::new(tx, policy, TurnPrimary::default());
    assert!(
        !decider.approve(&fetch, &there).await,
        "`fetch:https+a.example` does not allow `fetch:https+b.example`: it asks, \
         and nobody answers"
    );
}
