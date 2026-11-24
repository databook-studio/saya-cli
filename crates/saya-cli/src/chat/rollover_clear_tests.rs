//! `/clear` and the rollover's silence: the cleared session starts its next run
//! with nothing, and no rollover asks anybody — nor contends for the folder the
//! run it ended has let go.

#![cfg(unix)]

use std::sync::Arc;

use saya_store::RunLink;

use super::ChatOutcome;
use super::compose_fixture;
use super::operator_front_fixture;
use super::rollover::{Ending, Rollover, RolloverCause};
use super::rollover_fixture::{job_id, last_body, messages, next_run, open, transcript, turn};
use crate::commands::coding::parent_turn_provider::{Reply, Scripted};

/// Test 3: `/clear` rolls with an empty seed and an empty transcript.
#[tokio::test]
async fn clear_rolls_with_an_empty_seed_and_an_empty_transcript() {
    let provider = Scripted::start_with_script(vec![
        Reply::Text("first answer"),
        Reply::Text("the cleared run answered"),
    ]);
    let fixture = compose_fixture::in_place("rollover-clear", &provider.endpoint).await;
    let front = operator_front_fixture::non_interactive();
    let session = open(fixture, &front, "roll-clear").await;
    let mut transcript = transcript("roll-clear");
    let answer = turn(session.run.job(), "first message").await;
    assert!(transcript.settle("first message", &answer, &ChatOutcome::Answered));

    let rollover = Rollover::new(Arc::clone(&session.application));
    let rolled = rollover
        .roll(
            Ending::new(session.run),
            RolloverCause::Clear,
            next_run(front.front(), "roll-clear-2"),
            &mut transcript,
        )
        .await
        .expect("the cleared session prepares the next run");

    assert!(
        transcript.turns().is_empty(),
        "`/clear` clears the transcript: {:?}",
        transcript.turns()
    );
    assert_eq!(
        transcript.record().run.clone(),
        Some(RunLink {
            job: job_id("roll-clear-2"),
            epoch: rolled.job().epoch(),
        }),
        "the cleared session's record links the run it prepared"
    );

    let answered = turn(rolled.job(), "the first message after clear").await;
    assert_eq!(answered, "the cleared run answered");
    let sent = messages(&last_body(&provider));
    assert_eq!(
        sent.len(),
        2,
        "the cleared run's first request is its system message and the new prompt: {sent:?}"
    );
    assert!(
        sent[1].0 == "user" && sent[1].1.contains("the first message after clear"),
        "the cleared run sends the new prompt: {sent:?}"
    );
    assert!(
        !sent
            .iter()
            .any(|(_, text)| text == "first message" || text == "first answer"),
        "`/clear` seeds the next run with nothing: {sent:?}"
    );
}

/// Test 4: a rollover asks nobody — the session's mode is the consent — and it
/// never meets `InPlaceRootBusy`: the run that ends has let the folder go before
/// the next job is created.
#[tokio::test]
async fn a_rollover_asks_nobody_and_never_contends_for_the_folder() {
    let provider = Scripted::start();
    let fixture = compose_fixture::in_place("rollover-quiet", &provider.endpoint).await;
    // A front with a person attached: anything asked would be delivered here.
    let front = operator_front_fixture::interactive();
    let session = open(fixture, &front, "roll-quiet").await;
    let mut transcript = transcript("roll-quiet");

    let rollover = Rollover::new(Arc::clone(&session.application));
    let second = rollover
        .roll(
            Ending::new(session.run),
            RolloverCause::Provider,
            next_run(front.front(), "roll-quiet-2"),
            &mut transcript,
        )
        .await
        .expect("a rollover takes the folder's lease the ended run released");
    assert!(
        second.job().holds_folder_lease(),
        "the prepared run holds the folder's lease the ended run let go"
    );
    let third = rollover
        .roll(
            Ending::new(second),
            RolloverCause::Privacy,
            next_run(front.front(), "roll-quiet-3"),
            &mut transcript,
        )
        .await
        .expect("a second rollover contends for nothing");
    assert!(
        third.job().holds_folder_lease(),
        "the third run holds the folder's lease"
    );

    assert_eq!(
        front.seen(),
        0,
        "a rollover puts no question to the front: consent is the session's mode"
    );
    assert_eq!(
        transcript.record().run.clone(),
        Some(RunLink {
            job: job_id("roll-quiet-3"),
            epoch: third.job().epoch(),
        }),
        "the record follows the run the last rollover prepared"
    );
}
