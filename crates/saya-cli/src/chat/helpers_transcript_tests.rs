//! A helper's work on the chat's own stream: its tool calls reach the parent
//! run's events stamped with the helper's id, the root's own calls stay
//! unstamped, and folding that one stream gives the transcript one helper
//! block per caller with its step count (`02` §3.6).
//!
//! The whole way, not one half: the stream is what a scripted helper run
//! actually emitted, so a forwarder that dropped the caller would fold into the
//! root's activity group and fail here.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use saya_agent::{AgentEvent, AgentEventSink, ApprovalPolicy};
use serde_json::json;

use super::compose::RunComposition;
use super::compose_fixture::{self, Composed, RootGuard};
use super::{Admission, ChatEvent, ChatFront, ChatSession, JobPort, Question, RunPort};
use crate::agent::tools::consent::Answer;
use crate::commands::coding::parent_turn_provider::{Reply, Scripted};
use crate::front::{Entry, Helper, HelperState, TranscriptView, apply};

/// Load ceiling, not a product bound: how long one scripted turn may take.
const RUN: Duration = Duration::from_secs(120);

/// The helper the scripted root delegates to.
const HELPER: &str = "helper-1";

/// One turn's stream, kept whole.
#[derive(Default)]
struct Recorder {
    events: Mutex<Vec<AgentEvent>>,
}

impl Recorder {
    fn events(&self) -> MutexGuard<'_, Vec<AgentEvent>> {
        self.events.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[async_trait]
impl AgentEventSink for Recorder {
    async fn emit(&self, event: AgentEvent) {
        self.events().push(event);
    }
}

/// The front that hands the run the recorder: one stream for the root and for
/// every helper the turn launches, which is the front a chat session is.
struct RecordingFront {
    recorder: Arc<Recorder>,
    ended: Mutex<Vec<ChatEvent>>,
}

impl RecordingFront {
    fn new(recorder: Arc<Recorder>) -> Self {
        Self {
            recorder,
            ended: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl ChatFront for RecordingFront {
    fn sink(&self) -> Arc<dyn AgentEventSink> {
        Arc::clone(&self.recorder) as Arc<dyn AgentEventSink>
    }

    async fn ask(&self, _question: &Question) -> Answer {
        Answer::Deny
    }

    fn emit(&self, event: ChatEvent) {
        self.ended
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event);
    }
}

/// The composed and opened run over one fixture, with the folder kept alive.
async fn compose_case(
    fixture: compose_fixture::Fixture,
    front: Arc<dyn ChatFront>,
    session: &'static str,
) -> (RunComposition, RootGuard) {
    let Composed {
        application,
        run,
        root,
    } = fixture.into_parts(front, session, ApprovalPolicy::Ask);
    let composition = RunComposition::compose(application, run)
        .await
        .expect("the session's run composes");
    composition.open().await.expect("the run opens");
    (composition, root)
}

/// One turn of the session, carrying the operator's own words: the session is
/// what states the stream a helper's calls join, as a chat does.
async fn turn(composition: &RunComposition, front: &Arc<RecordingFront>, message: &str) {
    let session = Arc::new(ChatSession::new(
        Arc::new(JobPort::opened(Arc::clone(composition.job()))) as Arc<dyn RunPort>,
        Arc::clone(front) as Arc<dyn ChatFront>,
    ));
    session.open().await;
    let admission = tokio::time::timeout(RUN, session.send(message.to_owned()))
        .await
        .expect("the turn settles");
    assert_eq!(admission, Admission::Ran, "the message ran as a turn");
}

/// The root asks for one helper; the helper makes one call of its own.
fn script() -> Vec<Reply> {
    vec![
        Reply::Call {
            name: "helpers",
            arguments: json!({
                "description": "look around",
                "prompt": "find the tracked file in this folder",
                "subagent_type": "explore",
                "agent_id": HELPER,
            }),
        },
        Reply::Call {
            name: "glob",
            arguments: json!({"pattern": "*"}),
        },
        Reply::Text("helper done"),
        Reply::Text("root done"),
    ]
}

/// A helper's call reaches the parent's stream with `caller` set, the root's
/// own calls keep it unset, and the fold gives one helper block with its step
/// count — never a step of the root's activity group.
#[tokio::test]
async fn a_helpers_calls_fold_into_one_block_of_their_own() {
    let provider = Scripted::start_with_script(script());
    let fixture = compose_fixture::in_place("helpers-transcript", &provider.endpoint).await;
    let recorder = Arc::new(Recorder::default());
    let front = Arc::new(RecordingFront::new(Arc::clone(&recorder)));
    let (composition, _root) = compose_case(
        fixture,
        Arc::clone(&front) as Arc<dyn ChatFront>,
        "session-transcript",
    )
    .await;
    turn(
        &composition,
        &front,
        "use a helper to find where the tracked file is",
    )
    .await;

    // The stream, as the parent run received it.
    let events = recorder.events().clone();
    let mut root_calls = Vec::new();
    let mut helper_calls = Vec::new();
    for event in &events {
        if let AgentEvent::ToolRequested { name, caller, .. } = event {
            match caller {
                Some(caller) => helper_calls.push((caller.clone(), name.clone())),
                None => root_calls.push(name.clone()),
            }
        }
    }
    assert_eq!(
        helper_calls,
        vec![(HELPER.to_owned(), "glob".to_owned())],
        "the helper's own call reaches the parent's stream with its caller: {events:?}"
    );
    assert_eq!(
        root_calls,
        vec!["helpers".to_owned()],
        "the root's own calls carry no caller, so their bytes are what they always were"
    );

    // And the transcript that stream folds into.
    let mut view = TranscriptView::new();
    view.begin_turn("use a helper to find where the tracked file is");
    for event in &events {
        apply(&mut view, &ChatEvent::Agent(event.clone()));
    }
    let entries = view.turns()[0].entries();
    let helpers: Vec<&Helper> = entries
        .iter()
        .filter_map(|entry| match entry {
            Entry::Helper(helper) => Some(helper),
            _ => None,
        })
        .collect();
    assert_eq!(helpers.len(), 1, "one helper is one block: {entries:?}");
    assert_eq!(helpers[0].id, HELPER, "the block names its caller");
    assert_eq!(
        helpers[0].steps, 1,
        "the block counts the steps the helper took: {entries:?}"
    );
    assert_eq!(
        helpers[0].state,
        HelperState::Done,
        "the block settles once the call that launched it returned: {entries:?}"
    );
    let steps = entries
        .iter()
        .find_map(|entry| entry.activity())
        .map(|activity| activity.steps.len())
        .unwrap_or(0);
    assert_eq!(
        steps, 1,
        "the root's group holds the root's own call and none of the helper's: {entries:?}"
    );
}
