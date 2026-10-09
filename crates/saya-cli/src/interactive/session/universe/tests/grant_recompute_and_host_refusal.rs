/// Every family grant at once cannot widen a restrictive posture:
/// read-only and never never consult grants, and Plan denies before the
/// approval match — every family stays hidden under each. The `sql:`
/// token rides along to pin that the SQL family behaves exactly as today
/// (it widens none of these families; its own tools ride the read surface).
#[test]
fn no_family_grant_widens_read_only_never_or_plan() {
    let project = worktree("grant-restrictive-families");
    let state = temp_dir("grant-restrictive-families-state");
    let universe = compose(&session_runtime(None), &project, &state);
    granted(
        &universe,
        &[
            "workspace-write",
            "scratch",
            "fetch:https+docs.example",
            "runner:touch",
            "interpreter:python3",
            "command:cargo",
            "sql:docs",
        ],
    );
    for (agent_mode, mode, can_obtain_approval, label) in [
        (
            AgentMode::Build,
            ApprovalPolicy::ReadOnly,
            true,
            "read-only",
        ),
        (AgentMode::Build, ApprovalPolicy::Never, true, "never"),
        (
            AgentMode::Plan,
            ApprovalPolicy::Ask,
            false,
            "plan, headless",
        ),
        (
            AgentMode::Plan,
            ApprovalPolicy::Ask,
            true,
            "plan, with a surface",
        ),
    ] {
        let names = advertised(&universe, agent_mode, mode, can_obtain_approval);
        for tool in [
            "workspace_write",
            "workspace_edit",
            "scratch_sql",
            "scratch_import",
            "http_fetch",
            "http_download",
            "run_program",
            "run_command",
            "render_chart",
        ] {
            assert!(
                !names.contains(&tool.to_string()),
                "{label} stays closed under every family grant: {tool} hidden: {names:?}"
            );
        }
    }
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// A grant recorded after the turn (by this turn's decider, through the
/// same live store the surfaces bind) recomputes into the next turn's
/// definitions: the availability decision reads the live store per turn,
/// never a copy taken once.
#[test]
fn a_grant_recorded_after_the_turn_recomputes_into_the_next_definitions() {
    let project = worktree("grant-recompute");
    let state = temp_dir("grant-recompute-state");
    let universe = compose(&session_runtime(None), &project, &state);
    let policy = granted(&universe, &[]);
    let names = advertised(&universe, AgentMode::Build, ApprovalPolicy::Ask, false);
    assert!(
        !names.contains(&"scratch_sql".to_string()),
        "no grant yet: the headless ask stays closed: {names:?}"
    );
    // The decider records the grant mid-session; the store is the one the
    // view is bound to, so the next turn's definitions see it.
    policy.grants().grant("scratch");
    let names = advertised(&universe, AgentMode::Build, ApprovalPolicy::Ask, false);
    assert!(
        names.contains(&"scratch_sql".to_string()),
        "the recorded grant recomputes into the next turn's definitions: {names:?}"
    );
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// The turn-boundary confirmation: a headless ask session whose live
/// policy holds `fetch:https+a.example` runs its turn with `http_download`
/// advertised, but a call naming a different host is denied at call time —
/// the engine builds the call's own token from the actual arguments and
/// only an exactly granted one pre-answers — with no prompt (headless
/// surfaces cannot read one) and the destination never written. The
/// granted host's call, by contrast, resolves `Allow` at the decider.
#[tokio::test]
async fn a_headless_ask_session_denies_a_foreign_host_call_under_its_fetch_grant() {
    use crate::agent::runtime::run_prompt_with_inputs;
    use crate::agent::turn_inputs::TurnInputs;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    struct ScriptedDownload {
        issued: AtomicBool,
    }
    #[async_trait::async_trait]
    impl saya_agent::ChatProvider for ScriptedDownload {
        fn name(&self) -> &str {
            "scripted-download"
        }
        async fn complete(
            &self,
            _request: saya_agent::ChatRequest,
        ) -> Result<saya_agent::ChatResponse, saya_agent::ProviderError> {
            if !self.issued.swap(true, Ordering::SeqCst) {
                let mut message = saya_agent::ChatMessage::text("assistant", "");
                message.tool_calls = vec![saya_agent::ToolCall {
                    id: "call-1".into(),
                    name: "http_download".into(),
                    arguments: serde_json::json!({
                        "url": "https://b.example/denied.bin",
                        "destination": "denied.bin"
                    }),
                }];
                Ok(saya_agent::ChatResponse::new(message))
            } else {
                Ok(saya_agent::ChatResponse::new(
                    saya_agent::ChatMessage::text("assistant", "done"),
                ))
            }
        }
    }
    struct RecordingSink(std::sync::Mutex<Vec<saya_agent::AgentEvent>>);
    #[async_trait::async_trait]
    impl saya_agent::AgentEventSink for RecordingSink {
        async fn emit(&self, event: saya_agent::AgentEvent) {
            self.0.lock().unwrap().push(event);
        }
    }

    let project = worktree("grant-fetch-process");
    let state = temp_dir("grant-fetch-process-state");
    let runtime = session_runtime(None);
    let universe = Arc::new(compose(&runtime, &project, &state));
    let policy = granted(&universe, &["fetch:https+a.example"]);
    let definitions = universe.definitions(
        AgentMode::Build,
        ApprovalPolicy::Ask,
        false,
        true,
        false,
        false,
    );
    let download = definitions
        .iter()
        .find(|definition| definition.name == "http_download")
        .expect("the held fetch grant advertises http_download with a workspace bound");
    // The turn's real decider, unable to prompt — the headless shape.
    let decider: Arc<dyn saya_agent::ApprovalDecider> = Arc::new(
        crate::prompt_approval::TerminalApproval::from_session_with_cancellation(
            policy,
            false,
            universe.primary.clone(),
            universe.approval_facts(&runtime),
            None,
            saya_agent::CancellationToken::new(),
        ),
    );
    // The granted host's call pre-answers; the foreign host's does not —
    // one fetch token authorizes one destination, judged per call.
    assert!(
        decider
            .approve(
                download,
                &serde_json::json!({"url": "https://a.example/ok.bin", "destination": "ok.bin"}),
            )
            .await,
        "the granted destination's call is allowed at the decider"
    );
    assert!(
        !decider
            .approve(
                download,
                &serde_json::json!({
                    "url": "https://b.example/denied.bin",
                    "destination": "denied.bin"
                }),
            )
            .await,
        "a different host under the same family grant is denied at the decider"
    );
    let inputs = TurnInputs {
        ai: runtime.resolved.ai.clone(),
        provider: Box::new(ScriptedDownload {
            issued: AtomicBool::new(false),
        }),
        registry: crate::connection::ConnectionRegistry::new(""),
        failures: Vec::new(),
    };
    let sink = RecordingSink(std::sync::Mutex::new(Vec::new()));
    let output = run_prompt_with_inputs(
        &runtime,
        inputs,
        "download the file",
        ApprovalPolicy::Ask,
        false,
        false,
        Vec::new(),
        &sink,
        saya_agent::CancellationToken::new(),
        None,
        Some(Arc::clone(&decider)),
        None,
        Some(Arc::clone(&universe)),
        AgentMode::Build,
        None,
        None,
    )
    .await;
    output.expect("the turn completes: the foreign call was denied, not run");
    {
        let events = sink.0.lock().unwrap();
        assert!(
            events.iter().any(|event| matches!(
                event,
                saya_agent::AgentEvent::ToolDenied { name, .. }
                    if name.as_str() == "http_download"
            )),
            "the foreign-host call was denied at the call boundary: {events:?}"
        );
    }
    assert!(
        !project.join("denied.bin").exists(),
        "the denied call never wrote its destination"
    );
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}
