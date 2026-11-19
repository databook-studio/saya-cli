//! The operator a job started from the TUI puts its egressing calls to: a
//! `shell` host run — the root's or a helper's — a `web_fetch`, and a
//! plan-first job's `submit_plan`. The subject line names the caller, and the
//! one thing the operator really answers for is the call's own target: the
//! exact command and working directory, or the URL about to be fetched. No
//! answer is remembered between calls — a refusal is a normal denial.
//!
//! A shell call reaches this only on the host lane, never when it named the
//! sandbox; a fetch reaches it under `ask` and never under `bypass`.
//!
//! Both targets are model-authored, so they are sanitised before they reach
//! the operator's screen: a raw ESC, CR or NUL could move the cursor or rewrite
//! the line being read, and an invisible format character could hide part of
//! what is being approved. The sanitiser and the length bound are the shell's
//! own (`agent::tools::shell_policy`), so this surface and every other one that
//! puts such a call to a human show the same text. A target longer than the
//! prompt shows is refused outright — the operator never answers for text it
//! cannot read in full.

use super::CodingMessage;
use super::wait_policy::{WaitPolicy, await_answer};
use crate::agent::tools::consent::Operator;
use crate::agent::tools::shell_policy::sanitise_prompt_text;
use async_trait::async_trait;
use saya_agent::{ApprovalChoice, CancellationToken};
use serde_json::Value;
use std::sync::mpsc;
use tokio::sync::oneshot;

/// The longest command, working directory or URL this prompt shows, in
/// characters. A longer one is refused before the operator is asked: the cut
/// tail of a command could carry anything, and the operator would be approving
/// text it never saw. The panel's own bound — its prompt is a fixed shape, so
/// it must refuse rather than paint an unbounded target.
const MAX_PROMPT_CHARS: usize = 2048;

/// The operator of a job this TUI started: the panel's own queue and the job's
/// token, so the question reaches the surface a person is looking at.
pub(super) struct ChatOperator {
    sender: mpsc::SyncSender<CodingMessage>,
    token: CancellationToken,
}

impl ChatOperator {
    pub(super) fn new(sender: mpsc::SyncSender<CodingMessage>, token: CancellationToken) -> Self {
        Self { sender, token }
    }
}

/// The line that states what is being approved, or `None` when the arguments
/// carry no target this surface can show — which is a refusal, never a prompt
/// with nothing in it. A shell call states its command and where it runs; a
/// fetch states the URL it would send a request to; a plan-first job's
/// `submit_plan` states the plan itself.
fn target(arguments: &Value) -> Option<String> {
    if let Some(command) = arguments.get("command").and_then(Value::as_str) {
        let workdir = arguments
            .get("workdir")
            .and_then(Value::as_str)
            .unwrap_or(".");
        if command.chars().count() > MAX_PROMPT_CHARS || workdir.chars().count() > MAX_PROMPT_CHARS
        {
            return None;
        }
        return Some(format!(
            "in: {}\n$ {}",
            sanitise_prompt_text(workdir),
            sanitise_prompt_text(command)
        ));
    }
    if let Some(plan) = arguments.get("plan").and_then(Value::as_str) {
        // The plan gate caps a plan at the same number of characters this
        // prompt can paint, so the check is a shape guard rather than a bound
        // the operator ever meets: a longer plan was refused before it reached
        // here.
        if plan.chars().count() > MAX_PROMPT_CHARS {
            return None;
        }
        return Some(sanitise_prompt_text(plan));
    }
    let url = arguments.get("url").and_then(Value::as_str)?;
    if url.chars().count() > MAX_PROMPT_CHARS {
        return None;
    }
    Some(format!("GET {}", sanitise_prompt_text(url)))
}

#[async_trait]
impl Operator for ChatOperator {
    async fn ask(&self, subject: &str, arguments: &Value) -> bool {
        // Never ask about a target the operator cannot read in full, and never
        // ask with no target at all: a refusal is the safe answer, and the
        // model is free to send a shorter one.
        let Some(target) = target(arguments) else {
            return false;
        };
        let (respond, answer) = oneshot::channel();
        // The chat's prompt answers with the choice its modal records; a
        // question the prompt cannot take — a full queue, one already open —
        // is refused, never held.
        if self
            .sender
            .try_send(CodingMessage::Question {
                subject: subject.to_owned(),
                target,
                respond,
            })
            .is_err()
        {
            return false;
        }
        // A person is answering this one: no deadline, but the job's token
        // still ends the wait. `AllowSession` admits this call only — what a
        // remembered grant means for a job's calls is not decided here, so
        // nothing is stored.
        let choice = tokio::select! {
            _ = self.token.cancelled() => return false,
            choice = await_answer(answer, WaitPolicy::Attached) => choice,
        };
        matches!(
            choice,
            Some(ApprovalChoice::AllowOnce | ApprovalChoice::AllowSession { .. })
        )
    }
}
