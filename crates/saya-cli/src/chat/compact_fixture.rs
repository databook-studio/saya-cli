//! The session-side pieces a compaction test drives: a run open over the
//! fixture's folder, a record naming the model whose window the trigger reads,
//! and the readings a test takes of a settled turn.
//!
//! A compaction is the one place two runs of one session meet, so the folder and
//! the application both runs open over are kept alive for as long as the test
//! holds the fixture ([`super::rollover_fixture`] holds them).

use std::sync::Arc;

use saya_agent::{AgentOutput, NoopEventSink};
use saya_store::{RedactedSession, RedactedTurn};

use super::compact_provider::Reporting;
use super::compose_fixture;
use super::operator_front_fixture;
use super::rollover_fixture::{self, Open, RUN};
use super::transcript::Transcript;
use crate::commands::coding::NativeCodingJob;

/// One session's run, open over the fixture's folder, against `provider`.
pub(super) async fn open(
    case: &str,
    provider: &Reporting,
    front: &operator_front_fixture::Fixture,
    session: &str,
) -> Open {
    let fixture = compose_fixture::in_place(case, &provider.endpoint).await;
    rollover_fixture::open(fixture, front, session).await
}

/// The record a session's conversation starts in, naming the model whose window
/// the trigger reads. An unknown model is a session with no known window.
pub(super) fn record(id: &str, model: &str) -> Transcript {
    Transcript::from_record(RedactedSession {
        id: id.to_owned(),
        model: model.to_owned(),
        ..RedactedSession::default()
    })
}

/// Runs one turn and settles it answered into the transcript, reporting the
/// output the session's compaction decision reads.
pub(super) async fn settled(
    job: &Arc<NativeCodingJob>,
    transcript: &mut Transcript,
    prompt: &str,
) -> AgentOutput {
    let output = tokio::time::timeout(RUN, job.run_next_turn(&NoopEventSink, prompt))
        .await
        .expect("the turn settles")
        .unwrap_or_else(|error| panic!("the turn {prompt:?} runs: {error}"));
    assert!(
        transcript.settle(prompt, &output.answer, &super::ChatOutcome::Answered),
        "an answered turn is recorded in the transcript"
    );
    output
}

/// The settled pairs a transcript holds, oldest first.
pub(super) fn pairs(transcript: &Transcript) -> Vec<(String, String)> {
    transcript
        .turns()
        .into_iter()
        .map(|turn| (turn.prompt, turn.answer))
        .collect()
}

/// The turns a transcript holds, as the record keeps them: what a compaction
/// plans over.
pub(super) fn turns(transcript: &Transcript) -> Vec<RedactedTurn> {
    transcript.record().turns.clone()
}
