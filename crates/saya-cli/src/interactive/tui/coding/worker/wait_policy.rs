//! How a question to the operator waits for an answer. The question is queued
//! to the panel first, so a full or closed queue is refused before any wait
//! begins; the policy then decides how long the answer is awaited. A detached
//! surface learns the answer on its next drain, so its question must not hold
//! the run open; a surface with a person attached has no deadline of its own.
//!
//! Every answer is the run's own: nothing is remembered between questions, so
//! a run outside the sandbox asks again.

use super::CodingMessage;
use saya_agent::CancellationToken;
use std::sync::mpsc;

/// How long a question to the operator waits for an answer.
///
/// The job panel is `PANEL`: a detached surface, which learns the answer on
/// its next drain, so a question nobody is watching must not hold the run open.
/// A job this TUI started is `Attached`: the chat front end puts the question
/// in its own approval prompt, where a person is present, so only the answer or
/// cancellation ends the wait.
pub(super) enum WaitPolicy {
    /// A person is present and answers as soon as they decide: the wait lasts
    /// until they answer, or until the run is cancelled.
    Attached,
    /// The surface reports the answer on a later drain, so the wait gives up
    /// after this long and the question is refused.
    Detached(std::time::Duration),
}

impl WaitPolicy {
    /// The job panel's policy: it drains the answer on its next frame, so an
    /// unanswered question expires after 120 s rather than holding the run.
    pub(super) const PANEL: WaitPolicy = WaitPolicy::Detached(std::time::Duration::from_secs(120));
}

/// Puts one question to the panel and waits for the operator's answer as
/// `policy` allows: a detached wait ends at its deadline, an attached one waits
/// for the person. A full or closed queue, a cancelled run, an expired
/// deadline, a closed answer channel, and a `false` answer all refuse. No
/// answer is ever remembered: a run outside the sandbox asks again.
pub(super) async fn ask_operator(
    sender: &mpsc::SyncSender<CodingMessage>,
    token: &CancellationToken,
    text: String,
    policy: WaitPolicy,
) -> bool {
    let (respond, answer) = tokio::sync::oneshot::channel();
    if sender
        .try_send(CodingMessage::Approval { text, respond })
        .is_err()
    {
        return false;
    }
    tokio::select! {
        _ = token.cancelled() => false,
        approved = await_answer(answer, policy) => approved == Some(true),
    }
}

/// The answer, as far as `policy` allows it to arrive: `Attached` waits for the
/// person however long they take, `Detached(deadline)` gives up after
/// `deadline`. A closed answer channel is no answer at all.
pub(super) async fn await_answer<T>(
    answer: tokio::sync::oneshot::Receiver<T>,
    policy: WaitPolicy,
) -> Option<T> {
    match policy {
        WaitPolicy::Attached => answer.await.ok(),
        WaitPolicy::Detached(deadline) => match tokio::time::timeout(deadline, answer).await {
            Ok(answer) => answer.ok(),
            Err(_) => None,
        },
    }
}
