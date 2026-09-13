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

use super::{ChannelApproval, StreamMsg};
use crate::agent::tools::DatabaseTools;
use crate::approval_facts::ApprovalFacts;
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
    let decider = ChannelApproval::new(
        tx,
        SessionPolicy::new(ApprovalPolicy::Ask),
        primary.clone(),
        crate::approval_facts::ApprovalFacts::default(),
        None,
    );
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
        crate::approval_facts::ApprovalFacts::default(),
        None,
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
    let turn_three = ChannelApproval::new(
        tx,
        policy.clone(),
        TurnPrimary::default(),
        crate::approval_facts::ApprovalFacts::default(),
        None,
    );
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
    let turn_four = ChannelApproval::new(
        tx,
        policy.clone(),
        TurnPrimary::default(),
        crate::approval_facts::ApprovalFacts::default(),
        None,
    );
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
    let decider = ChannelApproval::new(
        tx,
        policy.clone(),
        TurnPrimary::default(),
        crate::approval_facts::ApprovalFacts::default(),
        None,
    );
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
    let decider = ChannelApproval::new(
        tx,
        policy.clone(),
        TurnPrimary::default(),
        crate::approval_facts::ApprovalFacts::default(),
        None,
    );
    assert!(
        decider.approve(&granted, &granted_args).await,
        "the granted shape runs without asking"
    );
    // A different shape still asks — and a deny there denies.
    let (tx, mut rx) = unbounded_channel();
    let decider = ChannelApproval::new(
        tx,
        policy.clone(),
        TurnPrimary::default(),
        crate::approval_facts::ApprovalFacts::default(),
        None,
    );
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
    let decider = ChannelApproval::new(
        tx,
        policy.clone(),
        TurnPrimary::default(),
        crate::approval_facts::ApprovalFacts::default(),
        None,
    );
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
    let decider = ChannelApproval::new(
        tx,
        policy.clone(),
        TurnPrimary::default(),
        crate::approval_facts::ApprovalFacts::default(),
        None,
    );
    assert!(
        !decider.approve(&fetch, &there).await,
        "`fetch:https+a.example` does not allow `fetch:https+b.example`: it asks, \
         and nobody answers"
    );
}

/// The session journal's `prompt` properties, driven through the one ask
/// surface a test can drive (the TUI's channel; the terminal decider shares
/// the operation, `record_prompt_answer`):
///
/// - a first `[s]` grant writes exactly one line — `source: "prompt"` — and
///   the line is already on disk when `approve` returns true, which is the
///   moment the call it allowed is allowed to run: the write precedes the
///   run gate opening, so the record is meaningful;
/// - a second grant of the same token writes none: the store answers
///   "already", and the journal-once hook is that answer.
#[tokio::test]
async fn a_prompted_grant_journals_once_before_the_call_it_allowed_runs() {
    let dir = std::env::temp_dir().join(format!("saya-tui-journal-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("state dir creates");
    let journal = std::sync::Arc::new(saya_store::SessionJournal::open(&dir));
    let (tx, mut rx) = unbounded_channel();
    let decider = ChannelApproval::new(
        tx,
        SessionPolicy::new(ApprovalPolicy::Ask),
        TurnPrimary::default(),
        crate::approval_facts::ApprovalFacts::default(),
        Some(journal.clone()),
    );
    let tool = crate::interactive::session_definitions::workspace_write();
    let arguments = serde_json::json!({"path": "notes.md", "content": "hello"});
    let answerer = tokio::spawn(async move {
        if let Some(StreamMsg::ApprovalRequest { respond, grant, .. }) = rx.recv().await {
            assert_eq!(grant.as_deref(), Some("workspace-write"));
            let _ = respond.send(ApprovalChoice::AllowSession {
                token: grant.expect("the ask offered a token"),
            });
        }
    });
    assert!(
        decider.approve(&tool, &arguments).await,
        "the user's session grant allows the call that asked"
    );
    answerer.await.expect("the answerer completes");
    // The line already exists at the moment the grant opened the run gate:
    // this is the ordering property, pinned at the only instant the test can
    // observe — the call it allowed is about to run.
    assert_eq!(
        journal.read().expect("the journal reads"),
        vec![saya_store::JournalEvent::Granted {
            token: "workspace-write".to_owned(),
            source: saya_store::GrantSource::Prompt,
        }],
        "one line, source prompt, written before the call it allowed runs"
    );
    // The second call of the same shape resolves Allow through the grant —
    // no ask, no new record, no line.
    assert!(
        decider.approve(&tool, &arguments).await,
        "the granted token pre-answers the next call of the same shape"
    );
    assert_eq!(
        journal.read().expect("the journal reads").len(),
        1,
        "a second grant of the same token writes none"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A journal write that fails must not take the session down — the user's
/// `[s]` stands and the call it allowed runs — and must not fail silently:
/// the decider says the missing audit line into the transcript.
#[tokio::test]
async fn a_failed_journal_write_says_so_and_does_not_take_the_call_down() {
    let dir = std::env::temp_dir().join(format!("saya-tui-journal-fail-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("state dir creates");
    // The journal path is a directory, so every append fails.
    std::fs::create_dir_all(dir.join("journal.ndjson")).expect("the block is made");
    let journal = std::sync::Arc::new(saya_store::SessionJournal::open(&dir));
    let (tx, mut rx) = unbounded_channel();
    let decider = ChannelApproval::new(
        tx,
        SessionPolicy::new(ApprovalPolicy::Ask),
        TurnPrimary::default(),
        crate::approval_facts::ApprovalFacts::default(),
        Some(journal),
    );
    let tool = crate::interactive::session_definitions::workspace_write();
    let arguments = serde_json::json!({"path": "notes.md", "content": "hello"});
    let answerer = tokio::spawn(async move {
        if let Some(StreamMsg::ApprovalRequest { respond, grant, .. }) = rx.recv().await {
            assert_eq!(grant.as_deref(), Some("workspace-write"));
            let _ = respond.send(ApprovalChoice::AllowSession {
                token: grant.expect("the ask offered a token"),
            });
        }
        // Then the warning the decider said arrives on the same channel.
        match rx.recv().await {
            Some(StreamMsg::Notice(warning)) => Some(warning),
            Some(_) => panic!("the decider said something other than the journal warning"),
            None => panic!("the decider said nothing"),
        }
    });
    assert!(
        decider.approve(&tool, &arguments).await,
        "the consent stands: a failed audit write does not revoke it"
    );
    let warning = answerer
        .await
        .expect("the answerer completes")
        .expect("the decider said the warning");
    assert!(
        warning.to_lowercase().contains("journal"),
        "the warning names the journal: {warning}"
    );
    // The grant is in force: the next call of the same shape resolves Allow
    // through it — the session carries on.
    assert!(
        decider.approve(&tool, &arguments).await,
        "the granted token pre-answers the next call: the session carries on"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
