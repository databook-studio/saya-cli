/// Deny wording end to end: the refused call reaches the model in the deny list’s own words.
/// Moved byte-identical from the hub; no snapshots involved.
use super::super::ChannelApproval;
use super::support::composed_facts;
use crate::approval_facts::ApprovalFacts;
use crate::grant_token::TurnPrimary;
use crate::prompt_approval::TerminalApproval;
use saya_agent::{ApprovalDecider, ApprovalPolicy, SessionPolicy};
use std::collections::VecDeque;
use tokio::sync::mpsc::unbounded_channel;

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
