//! D3 — a turn the engine refuses before admission is a budget stop, not an
//! answer.
//!
//! The old body pinned this from inside one question
//! (`agent/runtime_preflight_tests.rs::a_too_small_known_window_refuses_before_any_provider_request`):
//! a too-small known window refuses before any provider request — zero requests,
//! `truncated`, exactly one typed `RequestContextWindow` stop — and the question
//! reads that stop as its own budget stop
//! (`…::the_shared_question_scope_survives_a_preflight_refusal`: "the token's
//! first reason is the preflight budget").
//!
//! On the live path the refusal is unchanged (the engine latches it on the
//! turn's token, `saya-agent/src/loop_runner/admission.rs`), but a turn that
//! ends that way still reported [`ChatOutcome::Answered`]: the turn's own stop
//! was never stated, so the surface said "answered" about a turn that sent no
//! request at all. This is the same refusal, read back as the turn's stop and
//! as the chat outcome — the reading `saya ask`'s `Stopped` arm already
//! transcribes onto the question's token.

use std::time::Duration;

use saya_agent::{AgentEvent, ApprovalPolicy, BudgetDimension};

use super::compose::RunComposition;
use super::compose_fixture::{self, Ceilings};
use super::{Budget, ChatOutcome, operator_front_fixture};
use crate::commands::coding::TurnStop;
use crate::commands::coding::parent_turn_provider::{Reply, Scripted};

/// Load ceiling, not a product bound: how long one scripted turn may take.
const RUN: Duration = Duration::from_secs(120);

/// The window a chat request cannot fit in: 2048 clears nothing — the request's
/// own prompt plus the output reservation (512) leaves 512 tokens, and the
/// session's prompt and tools are far larger.
const REFUSING_WINDOW: u64 = 2_048;

/// The output reservation the refusing window must still leave room for, so the
/// refusal is the preflight's and not a composition error.
const OUTPUT_RESERVE: u32 = 512;

/// The front's own sink: every event the turn emitted, in order.
#[derive(Default, Clone)]
struct Recording(std::sync::Arc<std::sync::Mutex<Vec<AgentEvent>>>);

impl Recording {
    fn events(&self) -> Vec<AgentEvent> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn budget_stops(&self) -> Vec<BudgetDimension> {
        self.events()
            .into_iter()
            .filter_map(|event| match event {
                AgentEvent::AnsweringBudgetStopped { dimension } => Some(dimension),
                _ => None,
            })
            .collect()
    }
}

#[async_trait::async_trait]
impl saya_agent::AgentEventSink for Recording {
    async fn emit(&self, event: AgentEvent) {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(event);
    }
}

#[tokio::test]
async fn a_turn_refused_before_admission_is_the_budget_stop_not_an_answer() {
    // The script names the answer the model would have given: a refusal that
    // never reaches the provider is the only way this reply stays unsent.
    let provider =
        Scripted::start_with_script(vec![Reply::Text("the answer that was never asked for")]);
    let fixture = compose_fixture::in_place_with_database_ceilings(
        "preflight-stop",
        &provider.endpoint,
        Ceilings {
            context_window_tokens: Some(REFUSING_WINDOW),
            max_output_tokens: Some(OUTPUT_RESERVE),
            ..Ceilings::default()
        },
    )
    .await;
    let front = operator_front_fixture::non_interactive();
    let composed = fixture.into_parts(
        front.front(),
        "session-preflight-stop",
        ApprovalPolicy::ReadOnly,
    );
    let composition = RunComposition::compose(composed.application, composed.run)
        .await
        .expect("the session's run composes");
    let _root = composed.root;
    composition.open().await.expect("the run opens");
    let sink = Recording::default();

    let turn = tokio::time::timeout(
        RUN,
        composition
            .job()
            .run_next_turn(&sink, "what is the total revenue?"),
    )
    .await
    .expect("the turn settles");
    let output = turn.expect("a preflight refusal is truncated output, not an error");
    assert!(
        output.truncated,
        "the refusal is partial output: {output:?}"
    );
    assert_eq!(
        provider.arrived(),
        0,
        "a too-small known window refuses before any provider request"
    );
    assert_eq!(
        sink.budget_stops(),
        vec![BudgetDimension::RequestContextWindow],
        "exactly one RequestContextWindow stop: {:?}",
        sink.events()
    );
    let stop = composition.job().turn_stop();
    assert_eq!(
        stop,
        Some(TurnStop::Budget {
            dimension: BudgetDimension::RequestContextWindow,
            message: crate::budget_wording::budget_partial_message(
                BudgetDimension::RequestContextWindow
            ),
        }),
        "the turn reads the preflight dimension, with the shared budget sentence"
    );
    assert_eq!(
        ChatOutcome::from_stop(stop, false),
        ChatOutcome::Stopped(Budget {
            dimension: BudgetDimension::RequestContextWindow
        }),
        "a turn that sent no request must not be reported as answered"
    );
}
