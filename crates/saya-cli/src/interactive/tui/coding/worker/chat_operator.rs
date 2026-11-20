//! The operator a job started from the TUI puts its calls to: a `shell` host
//! run — the root's or a helper's — a `web_fetch`, a plan-first job's
//! `submit_plan`, and a `write`/`edit` the job's mode says to ask about. The
//! subject line names the caller, and the one thing the operator really answers
//! for is the call's own target: the exact command and working directory, the
//! URL about to be fetched, or the path about to change.
//!
//! One answer outlives its call: `AllowSession` records the call's own
//! `shell:` grant in this job's store (see [`Operator::grants`]), which every
//! shell of the job reads through its prompt — after its deny rules and its
//! mode, never instead of them. A write's `workspace-write` grant is recorded
//! by the ask that made the question, from the answer this operator reports.
//! Nothing is persisted: the store is the operator's, and ends with the job. A
//! refusal is a normal denial.
//!
//! A shell call reaches this only on the host lane, never when it named the
//! sandbox; a fetch reaches it under `ask` and never under `bypass`; a write
//! reaches it only under `ask`, and only where the exact-write admission
//! already holds the capability.
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
use crate::agent::tools::consent::{Answer, Operator};
use crate::agent::tools::shell_policy::sanitise_prompt_text;
use async_trait::async_trait;
use saya_agent::{ApprovalChoice, CancellationToken, SessionGrants};
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
    /// This job's "yes for this run" store: the shell grants its operator has
    /// already answered `AllowSession` for. One per operator, so the root and
    /// every helper of the job share it, and it ends with the job's operator:
    /// process-lifetime, never persisted.
    grants: SessionGrants,
}

impl ChatOperator {
    pub(super) fn new(sender: mpsc::SyncSender<CodingMessage>, token: CancellationToken) -> Self {
        Self {
            sender,
            token,
            grants: SessionGrants::default(),
        }
    }
}

/// The line that states what is being approved, or `None` when the arguments
/// carry no target this surface can show — which is a refusal, never a prompt
/// with nothing in it. A shell call states its command and where it runs; a
/// fetch states the URL it would send a request to; a plan-first job's
/// `submit_plan` states the plan itself; and a write-shaped call states the
/// path it would change.
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
    if let Some(path) = arguments.get("path").and_then(Value::as_str) {
        // The workspace tools bound a path well below this, so the check is a
        // shape guard here too: the operator is never asked about a path it
        // cannot read in full.
        if path.chars().count() > MAX_PROMPT_CHARS {
            return None;
        }
        return Some(format!("path: {}", sanitise_prompt_text(path)));
    }
    let url = arguments.get("url").and_then(Value::as_str)?;
    if url.chars().count() > MAX_PROMPT_CHARS {
        return None;
    }
    Some(format!("GET {}", sanitise_prompt_text(url)))
}

#[async_trait]
impl Operator for ChatOperator {
    /// The job's own store, shared by every shell of the job through the prompt
    /// it is composed into.
    fn grants(&self) -> Option<SessionGrants> {
        Some(self.grants.clone())
    }

    async fn ask(&self, subject: &str, arguments: &Value) -> bool {
        // `AllowSession` remembers this call's own `shell:` grant in the job's
        // store and admits the call; a command that is not one plain segment
        // names no token, so it is admitted for this call only. `AllowOnce`
        // admits the call and remembers nothing. The write ask records its own
        // family token from the answer this reports.
        match self.answer(subject, arguments).await {
            Answer::Deny => false,
            Answer::Once => true,
            Answer::Session => {
                if let Some(token) = crate::grant_token::shell_token_for_host_route(arguments) {
                    self.grants.grant(&token);
                }
                true
            }
        }
    }

    async fn answer(&self, subject: &str, arguments: &Value) -> Answer {
        // Never ask about a target the operator cannot read in full, and never
        // ask with no target at all: a refusal is the safe answer, and the
        // model is free to send a shorter one.
        let Some(target) = target(arguments) else {
            return Answer::Deny;
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
            return Answer::Deny;
        }
        // A person is answering this one: no deadline, but the job's token
        // still ends the wait.
        let choice = tokio::select! {
            _ = self.token.cancelled() => return Answer::Deny,
            choice = await_answer(answer, WaitPolicy::Attached) => choice,
        };
        match choice {
            Some(ApprovalChoice::AllowSession { .. }) => Answer::Session,
            Some(ApprovalChoice::AllowOnce) => Answer::Once,
            _ => Answer::Deny,
        }
    }
}
