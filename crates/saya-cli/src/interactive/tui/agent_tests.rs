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
use std::collections::VecDeque;
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

/// A composition that carries what these tests' calls name: runner doors
/// over `bench`/`deploy`, the fetch member, and a bound workspace root —
/// the facts a session composes for those programs (U8: the suggestion
/// gates on the composition, so a test asserting an offer must stage the
/// capability it offers).
fn composed_facts() -> ApprovalFacts {
    ApprovalFacts {
        runner: Some(crate::approval_facts::RunnerFacts {
            runner_programs: vec!["bench".into(), "deploy".into()],
            interpreter_programs: Vec::new(),
            ..crate::approval_facts::RunnerFacts::default()
        }),
        fetch: Some(crate::approval_facts::FetchFacts {
            fetch_body_bytes: 61_440,
            fetch_seconds: 30,
            fetch_redirects: 5,
            download: None,
        }),
        workspace_root: Some(std::path::PathBuf::from("/home/user/proj")),
        ..ApprovalFacts::default()
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
        composed_facts(),
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
        composed_facts(),
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
        composed_facts(),
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
        composed_facts(),
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
        composed_facts(),
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
        composed_facts(),
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
        composed_facts(),
        None,
    );
    assert!(
        !decider.approve(&fetch, &there).await,
        "`fetch:https+a.example` does not allow `fetch:https+b.example`: it asks, \
         and nobody answers"
    );
}

/// A denied name preempts a held grant through the policy engine: the grant
/// resolves `Allow`, but the decider still refuses without asking — in every
/// mode, bypass included. The ask never renders (the channel is closed, so
/// an ask would deny), and the refusal the decider words for the loop is the
/// deny bytes, not the engine's generic denial.
#[tokio::test]
async fn a_denied_name_preempts_a_held_grant_without_asking() {
    use crate::interactive::session_definitions;
    for mode in [ApprovalPolicy::Ask, ApprovalPolicy::Bypass] {
        let tool = session_definitions::run_command();
        let arguments = serde_json::json!({"program": "curl"});
        let policy = SessionPolicy::new(mode);
        policy.grants().grant("command:curl");
        let mut facts = composed_facts();
        facts.host = Some(crate::approval_facts::HostFacts::for_tests());
        facts.denied_programs = vec!["curl".to_owned()];
        assert_eq!(
            policy.resolve(&tool.effect, Some("command:curl")),
            ApprovalDecision::Allow,
            "the held grant resolves Allow under {mode:?}: deny preempts after the engine"
        );
        // The terminal decider refuses without prompting.
        let terminal = TerminalApproval::from_session(
            policy.clone(),
            mode == ApprovalPolicy::Ask,
            TurnPrimary::default(),
            facts.clone(),
            None,
        );
        assert!(
            !terminal.approve(&tool, &arguments).await,
            "a denied name refuses under {mode:?} even with the grant held"
        );
        assert!(
            terminal
                .refusal_detail(&tool, &arguments)
                .is_some_and(|detail| detail.contains("deny list")
                    && detail.contains("allowed programs may still invoke it")),
            "the terminal decider words the denial with the typed refusal under {mode:?}"
        );
        // The TUI decider refuses the same way: a closed channel means an
        // ask would deny, and the preemption answers before any ask renders.
        let (tx, rx) = unbounded_channel();
        drop(rx);
        let channel = ChannelApproval::new(
            tx,
            policy.clone(),
            TurnPrimary::default(),
            facts.clone(),
            None,
        );
        assert!(
            !channel.approve(&tool, &arguments).await,
            "the TUI refuses a denied name under {mode:?} even with the grant held"
        );
        assert!(
            channel
                .refusal_detail(&tool, &arguments)
                .is_some_and(|detail| detail.contains("deny list")
                    && detail.contains("allowed programs may still invoke it")),
            "the TUI decider words the denial with the typed refusal under {mode:?}"
        );
    }
}

/// A denied call reaches the model through the loop in the deny list's own
/// words: the decider refuses, and the tool result carries the loop's pinned
/// denial prefix followed by the typed refusal — one refusal, in the right
/// words, from the right layer. A non-deny denial keeps the loop's generic
/// denial byte-identical (no decider wording appended).
#[tokio::test]
async fn a_denied_call_reaches_the_model_in_the_deny_list_s_words() {
    use crate::interactive::{session_definitions, session_deny};
    use saya_agent::{AgentLimits, CancellationToken, run_agent_with_sink};
    use std::sync::{Arc, Mutex};

    struct ScriptProvider {
        script: Mutex<VecDeque<saya_agent::ToolCall>>,
        requests: Mutex<Vec<saya_agent::ChatRequest>>,
    }

    #[async_trait::async_trait]
    impl saya_agent::ChatProvider for ScriptProvider {
        fn name(&self) -> &str {
            "script"
        }

        async fn complete(
            &self,
            _: saya_agent::ChatRequest,
        ) -> Result<saya_agent::ChatResponse, saya_agent::ProviderError> {
            unreachable!("the loop streams, not completes")
        }

        async fn stream(
            &self,
            request: saya_agent::ChatRequest,
            _: saya_agent::CancellationToken,
        ) -> Result<saya_agent::ProviderStream, saya_agent::ProviderError> {
            use futures_util::stream;
            self.requests
                .lock()
                .expect("the requests lock")
                .push(request);
            let events = match self.script.lock().expect("the script locks").pop_front() {
                Some(call) => vec![
                    Ok(saya_agent::ProviderEvent::ToolCalls(vec![call])),
                    Ok(saya_agent::ProviderEvent::Done),
                ],
                None => vec![
                    Ok(saya_agent::ProviderEvent::TextDelta("done".into())),
                    Ok(saya_agent::ProviderEvent::Done),
                ],
            };
            Ok(Box::pin(stream::iter(events)))
        }
    }

    struct DenyTools;

    #[async_trait::async_trait]
    impl saya_agent::ToolExecutor for DenyTools {
        async fn execute(
            &self,
            _: &str,
            _: serde_json::Value,
        ) -> Result<serde_json::Value, saya_agent::ToolError> {
            panic!("a denied call must never execute");
        }
    }

    struct Sink {
        events: Arc<Mutex<Vec<saya_agent::AgentEvent>>>,
    }

    #[async_trait::async_trait]
    impl saya_agent::AgentEventSink for Sink {
        async fn emit(&self, event: saya_agent::AgentEvent) {
            self.events.lock().expect("the sink locks").push(event);
        }
    }

    async fn drive(
        decider: &dyn ApprovalDecider,
    ) -> (
        Vec<saya_agent::AgentEvent>,
        String,
        Vec<saya_agent::ChatRequest>,
    ) {
        let tool = session_definitions::run_command();
        let arguments = serde_json::json!({"program": "curl"});
        let provider = ScriptProvider {
            script: Mutex::new(VecDeque::from([saya_agent::ToolCall {
                id: "call-1".into(),
                name: tool.name.clone(),
                arguments: arguments.clone(),
            }])),
            requests: Mutex::new(Vec::new()),
        };
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = Sink {
            events: events.clone(),
        };
        let request = saya_agent::AgentRequest {
            prompt: "run it".into(),
            profile_names: Vec::new(),
            model: "script".into(),
            system_prompt: None,
            history: Vec::new(),
            context_blocks: Vec::new(),
        };
        let output = run_agent_with_sink(
            &provider,
            &DenyTools,
            request,
            vec![tool],
            AgentLimits::default(),
            decider,
            &sink,
            CancellationToken::new(),
        )
        .await
        .expect("a denial is not a turn-ending error");
        let denied_text = output
            .events
            .iter()
            .filter_map(|event| match event {
                saya_agent::AgentEvent::ToolDenied { name, reason } if name == "run_command" => {
                    Some(reason.clone())
                }
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        let _ = arguments;
        (
            events.lock().expect("the events lock").clone(),
            denied_text,
            provider.requests.lock().expect("the requests lock").clone(),
        )
    }

    /// Every message's content of one captured request, joined — the brief is
    /// in there only insofar as the loop replays tool results as history, and
    /// this is how the test reads what the model saw next.
    fn request_texts(request: &saya_agent::ChatRequest) -> String {
        request
            .messages
            .iter()
            .map(|message| message.content.clone())
            .collect::<Vec<_>>()
            .join("\n")
    }

    let tool = session_definitions::run_command();
    let arguments = serde_json::json!({"program": "curl"});
    let mut facts = composed_facts();
    facts.host = Some(crate::approval_facts::HostFacts::for_tests());
    facts.denied_programs = vec!["curl".to_owned()];
    let (tx, rx) = unbounded_channel();
    drop(rx);
    let channel = ChannelApproval::new(
        tx,
        SessionPolicy::new(ApprovalPolicy::Ask),
        TurnPrimary::default(),
        facts.clone(),
        None,
    );
    let (events, text, requests) = drive(&channel).await;
    let denied_reason = events
        .iter()
        .find_map(|event| match event {
            saya_agent::AgentEvent::ToolDenied { name, reason } if name == "run_command" => {
                Some(reason.clone())
            }
            _ => None,
        })
        .expect("the denied call emits ToolDenied");
    assert!(
        denied_reason.contains("deny list")
            && denied_reason.contains("allowed programs may still invoke it"),
        "the user sees the typed refusal on ToolDenied: {denied_reason}"
    );
    // The model sees the same refusal as the tool result: the loop replays
    // the denied result into the next turn's history, which the next provider
    // request carries.
    let seen_by_model = requests
        .iter()
        .map(request_texts)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        seen_by_model.contains("tool call denied by approval policy"),
        "the loop's pinned denial prefix reaches the model: {seen_by_model}"
    );
    for pin in [
        "deny list",
        "stated at launch or in user config",
        "this is saya's refusal, not a program failure",
        "bounds only the program named in the ask",
        "allowed programs may still invoke it",
    ] {
        assert!(
            seen_by_model.contains(pin),
            "the model sees the typed refusal ({pin}): {seen_by_model}"
        );
    }
    // The executor's builder is the same wording the decider carried: one
    // refusal, from the right layer, with no second dialect to drift.
    assert!(
        seen_by_model.contains(&session_deny::denied_refusal("curl")),
        "the loop carries the deny builder's own bytes: {seen_by_model}"
    );
    // The decider-level wording the unit half pinned is the same bytes the
    // loop replayed: one refusal, not a paraphrase per surface.
    assert!(
        text.contains(&session_deny::denied_refusal("curl")),
        "the ToolDenied reason carries the deny builder's own bytes: {text}"
    );
    // And a mode denial with no decider wording keeps the generic denial
    // byte-identical: the loop appends nothing it was not given.
    let bare = TerminalApproval::new(
        ApprovalPolicy::Never,
        false,
        TurnPrimary::default(),
        ApprovalFacts::default(),
    );
    let bare_tool = session_definitions::run_command();
    let bare_arguments = serde_json::json!({"program": "make"});
    assert!(
        !bare.approve(&bare_tool, &bare_arguments).await,
        "never mode denies a non-denied program"
    );
    assert_eq!(
        bare.refusal_detail(&bare_tool, &bare_arguments),
        None,
        "a mode denial words nothing: the generic denial stands"
    );
    let _ = (tool, arguments);
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
        composed_facts(),
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
        composed_facts(),
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
