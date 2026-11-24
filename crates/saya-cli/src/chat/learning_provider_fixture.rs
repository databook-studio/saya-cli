//! The post-turn extraction provider a chat test states, and the sink it reads.
//!
//! A chat run learns after it answers through the provider the session's run
//! holds (`SessionRun::learning`), so a test of that pass states its own: one
//! that answers straight away, one that is held so the pass is observable
//! while it runs, and one that never answers at all. The gates are plain
//! atomics polled at a few milliseconds — no wait can be lost between the
//! test and the provider.
//!
//! [`composed`] is the whole live path those tests drive: a run over the
//! fixture's folder and seeded database, its memory on, the learning provider
//! the test states, opened and ready for its first turn.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use saya_agent::{
    AgentEvent, AgentEventSink, ApprovalPolicy, ChatMessage, ChatProvider, ChatRequest,
    ChatResponse, ProviderError, TokenUsage,
};
use saya_config::MemoryMode;
use serde_json::json;

use super::compose::RunComposition;
use super::compose_fixture::{self, Ceilings, RootGuard};
use super::operator_front_fixture;
use crate::commands::coding::parent_turn_provider::{Reply, Scripted as ScriptedTurn};

/// The usage every answering scripted extraction reports.
pub(super) fn usage() -> TokenUsage {
    TokenUsage::new(40, 10)
}

/// A reply that proposes nothing: the pass runs, reports, and resolves no
/// claim, so a test of the *call* needs no schema behind it.
const NO_PROPOSALS: &str = r#"{"proposals": []}"#;

/// How the scripted extraction answers.
enum Mode {
    /// Straight away, with [`USAGE`].
    Answering,
    /// Only after [`Scripted::release`], with [`USAGE`].
    Held,
    /// Not at all: the provider fails, so no response is ever received.
    Failing,
}

/// The extraction provider a composed chat run is stated with.
pub(super) struct Scripted {
    mode: Mode,
    arrived: AtomicUsize,
    released: AtomicBool,
    stop: Duration,
}

impl Scripted {
    /// One that answers every extraction call, reporting [`USAGE`].
    pub(super) fn answering() -> Arc<Self> {
        Arc::new(Self::with(Mode::Answering))
    }

    /// One whose call arrives and then waits for [`Self::release`].
    pub(super) fn held() -> Arc<Self> {
        Arc::new(Self::with(Mode::Held))
    }

    /// One that never answers: the extraction call has no response.
    pub(super) fn failing() -> Arc<Self> {
        Arc::new(Self::with(Mode::Failing))
    }

    fn with(mode: Mode) -> Self {
        Self {
            mode,
            arrived: AtomicUsize::new(0),
            released: AtomicBool::new(false),
            stop: Duration::from_millis(5),
        }
    }

    /// How many extraction calls arrived.
    pub(super) fn arrived(&self) -> usize {
        self.arrived.load(Ordering::Acquire)
    }

    /// Waits until the extraction call has arrived: the turn that sent it is
    /// still running, past the answer.
    pub(super) async fn wait_arrived(&self) {
        while self.arrived() == 0 {
            tokio::time::sleep(self.stop).await;
        }
    }

    /// Lets a held extraction answer.
    pub(super) fn release(&self) {
        self.released.store(true, Ordering::Release);
    }
}

#[async_trait]
impl ChatProvider for Scripted {
    fn name(&self) -> &str {
        "scripted-extraction"
    }

    async fn complete(&self, _request: ChatRequest) -> Result<ChatResponse, ProviderError> {
        self.arrived.fetch_add(1, Ordering::AcqRel);
        match self.mode {
            Mode::Answering => Ok(answered()),
            Mode::Held => {
                while !self.released.load(Ordering::Acquire) {
                    tokio::time::sleep(self.stop).await;
                }
                Ok(answered())
            }
            Mode::Failing => Err(ProviderError::Request(
                "the extraction never answered".into(),
            )),
        }
    }
}

fn answered() -> ChatResponse {
    let mut response = ChatResponse::new(ChatMessage::text("assistant", NO_PROPOSALS));
    response.usage = Some(usage());
    response
}

/// A composed chat run whose memory is on and whose learning provider is
/// `learning`: a scripted turn that reads the seeded database, the sink that
/// turn reports to, the folder that outlives it, and the scripted provider
/// itself — the turn's endpoint, which must outlive the run.
pub(super) async fn composed(
    case: &str,
    learning: Arc<dyn ChatProvider>,
    ceilings: Ceilings,
) -> (RunComposition, Recording, RootGuard, ScriptedTurn) {
    let provider = ScriptedTurn::start_with_script(vec![
        Reply::Call {
            name: "bounded_sql_query",
            arguments: json!({"sql": "SELECT name FROM users ORDER BY id ASC LIMIT 1"}),
        },
        Reply::Text("The users table holds two people, ada and grace."),
    ]);
    let fixture = compose_fixture::in_place_with_memory_and_ceilings(
        case,
        &provider.endpoint,
        MemoryMode::Assisted,
        ceilings,
    )
    .await;
    let front = operator_front_fixture::non_interactive();
    let mut composed = fixture.into_parts(front.front(), case, ApprovalPolicy::ReadOnly);
    // The run's own memory facts: the mode the fixture resolved, and the run
    // that can honour it — what decides whether this run learns at all.
    composed.run.memory = MemoryMode::Assisted;
    composed.run.memory_reachable = true;
    composed.run.learning = Some(learning);
    let composition = RunComposition::compose(composed.application, composed.run)
        .await
        .expect("the session's run composes");
    let root = composed.root;
    composition.open().await.expect("the run opens");
    (composition, Recording::default(), root, provider)
}

/// The front's own sink: every event the turn emitted, in order.
#[derive(Default, Clone)]
pub(super) struct Recording(Arc<std::sync::Mutex<Vec<AgentEvent>>>);

impl Recording {
    /// The events this sink has been handed so far.
    pub(super) fn events(&self) -> Vec<AgentEvent> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Every extraction usage report the stream carried.
    pub(super) fn extraction_usage(&self) -> Vec<TokenUsage> {
        self.events()
            .into_iter()
            .filter_map(|event| match event {
                AgentEvent::Usage {
                    call: saya_agent::UsageCall::Extraction,
                    usage,
                } => Some(usage),
                _ => None,
            })
            .collect()
    }

    /// The answering budget dimensions this stream reported.
    pub(super) fn budget_stops(&self) -> Vec<saya_agent::BudgetDimension> {
        self.events()
            .into_iter()
            .filter_map(|event| match event {
                AgentEvent::AnsweringBudgetStopped { dimension } => Some(dimension),
                _ => None,
            })
            .collect()
    }

    /// Whether the turn announced that learning was skipped.
    pub(super) fn learning_skipped(&self) -> bool {
        self.events()
            .iter()
            .any(|event| matches!(event, AgentEvent::KnowledgeLearningSkipped { .. }))
    }

    /// Whether the turn started a learning pass at all.
    pub(super) fn learning_started(&self) -> bool {
        self.events()
            .iter()
            .any(|event| matches!(event, AgentEvent::KnowledgeLearningStarted))
    }
}

#[async_trait]
impl AgentEventSink for Recording {
    async fn emit(&self, event: AgentEvent) {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(event);
    }
}
