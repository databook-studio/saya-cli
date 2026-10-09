/// The process-level confirmation, through the turn boundary: a headless
/// ask session whose live policy holds the workspace-write grant runs a
/// `workspace_write` call the model issues — the definitions the turn
/// builds include the granted family, the derived workspace-write permit
/// follows, and the file lands in the workspace. Without the grant the
/// same call is an unsupported tool call: the family stays hidden and the
/// turn fails closed.
#[tokio::test]
async fn a_headless_ask_session_runs_a_workspace_write_call_under_its_grant() {
    use crate::agent::runtime::run_prompt_with_inputs;
    use crate::agent::turn_inputs::TurnInputs;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    struct ScriptedWrite {
        answered_tool_call: AtomicBool,
    }
    #[async_trait::async_trait]
    impl saya_agent::ChatProvider for ScriptedWrite {
        fn name(&self) -> &str {
            "scripted-write"
        }
        async fn complete(
            &self,
            _request: saya_agent::ChatRequest,
        ) -> Result<saya_agent::ChatResponse, saya_agent::ProviderError> {
            if !self.answered_tool_call.swap(true, Ordering::SeqCst) {
                let mut message = saya_agent::ChatMessage::text("assistant", "");
                message.tool_calls = vec![saya_agent::ToolCall {
                    id: "call-1".into(),
                    name: "workspace_write".into(),
                    arguments: serde_json::json!({
                        "path": "grant-headless.txt",
                        "content": "written under the grant\n"
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

    let project = worktree("grant-process");
    let state = temp_dir("grant-process-state");
    let runtime = session_runtime(None);
    let universe = Arc::new(compose(&runtime, &project, &state));
    let policy = granted(&universe, &["workspace-write"]);
    // The turn's real decider: over the same live policy (a clone shares
    // the grant store), unable to prompt, carrying the session's own facts
    // and primary handle — the shape `session_request::run` builds.
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
    let inputs = TurnInputs {
        ai: runtime.resolved.ai.clone(),
        provider: Box::new(ScriptedWrite {
            answered_tool_call: AtomicBool::new(false),
        }),
        registry: crate::connection::ConnectionRegistry::new(""),
        failures: Vec::new(),
    };
    let output = run_prompt_with_inputs(
        &runtime,
        inputs,
        "write the file",
        ApprovalPolicy::Ask,
        false,
        false,
        Vec::new(),
        &RecordingSink(std::sync::Mutex::new(Vec::new())),
        saya_agent::CancellationToken::new(),
        None,
        Some(decider),
        None,
        Some(Arc::clone(&universe)),
        AgentMode::Build,
        None,
        None,
    )
    .await;
    match output {
        Ok(_) => {}
        Err(error) => {
            panic!("the granted workspace_write call must run, never be unsupported: {error:?}")
        }
    }
    assert!(
        project.join("grant-headless.txt").exists(),
        "the granted call wrote through the workspace seam"
    );
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// The closed shape at the process level: the same headless ask turn with
/// NO grant keeps the family hidden — the model's `workspace_write` call
/// fails validation as an unknown tool and is fed back as a tool result,
/// never executed, with the workspace untouched. No grant, no
/// advertisement, no execution.
#[tokio::test]
async fn a_headless_ask_session_without_the_grant_refuses_the_workspace_write_call() {
    use crate::agent::runtime::run_prompt_with_inputs;
    use crate::agent::turn_inputs::TurnInputs;
    use std::sync::Arc;

    struct ScriptedWrite;
    #[async_trait::async_trait]
    impl saya_agent::ChatProvider for ScriptedWrite {
        fn name(&self) -> &str {
            "scripted-write"
        }
        async fn complete(
            &self,
            _request: saya_agent::ChatRequest,
        ) -> Result<saya_agent::ChatResponse, saya_agent::ProviderError> {
            let mut message = saya_agent::ChatMessage::text("assistant", "");
            message.tool_calls = vec![saya_agent::ToolCall {
                id: "call-1".into(),
                name: "workspace_write".into(),
                arguments: serde_json::json!({
                    "path": "grant-headless.txt",
                    "content": "must not be written\n"
                }),
            }];
            Ok(saya_agent::ChatResponse::new(message))
        }
    }
    struct RecordingSink(std::sync::Mutex<Vec<saya_agent::AgentEvent>>);
    #[async_trait::async_trait]
    impl saya_agent::AgentEventSink for RecordingSink {
        async fn emit(&self, event: saya_agent::AgentEvent) {
            self.0.lock().unwrap().push(event);
        }
    }

    let project = worktree("grant-process-closed");
    let state = temp_dir("grant-process-closed-state");
    let runtime = session_runtime(None);
    let universe = Arc::new(compose(&runtime, &project, &state));
    let policy = saya_agent::SessionPolicy::new(ApprovalPolicy::Ask);
    universe.bind_grants(policy.grants());
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
    let inputs = TurnInputs {
        ai: runtime.resolved.ai.clone(),
        provider: Box::new(ScriptedWrite),
        registry: crate::connection::ConnectionRegistry::new(""),
        failures: Vec::new(),
    };
    let sink = RecordingSink(std::sync::Mutex::new(Vec::new()));
    let output = run_prompt_with_inputs(
        &runtime,
        inputs,
        "write the file",
        ApprovalPolicy::Ask,
        false,
        false,
        Vec::new(),
        &sink,
        saya_agent::CancellationToken::new(),
        None,
        Some(decider),
        None,
        Some(Arc::clone(&universe)),
        AgentMode::Build,
        None,
        None,
    )
    .await;
    // The turn itself completes: an unadvertised call is fed back as a
    // failed tool result, never executed.
    output.expect("the closed turn completes; the call was refused, not run");
    {
        let events = sink.0.lock().unwrap();
        assert!(
            events.iter().any(|event| matches!(
                event,
                saya_agent::AgentEvent::ToolCompleted { name, .. }
                    if name.as_str() == "workspace_write"
            )),
            "the refused call was seen and fed back as a tool result: {events:?}"
        );
    }
    assert!(
        !project.join("grant-headless.txt").exists(),
        "the workspace was never written"
    );
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}
