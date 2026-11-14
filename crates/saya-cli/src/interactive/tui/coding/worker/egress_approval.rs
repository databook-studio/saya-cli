//! The panel's approver for every egressing call the coding panel puts to its
//! operator: a `shell` host run — the root's or a helper's — and a
//! `web_fetch`. The subject line names the caller, and the one thing the
//! operator really answers for is the call's own target:
//! the exact command and working directory, or the URL about to be fetched. No
//! answer is remembered between calls — a refusal is a normal denial.
//!
//! A shell call reaches this only on the host lane, never when it named the
//! sandbox; a fetch reaches it under `ask` and never under `bypass`.
//!
//! Both targets are model-authored, so they are sanitised before they reach the
//! operator's screen: a raw ESC, CR or NUL could move the cursor or rewrite the
//! line being read, and an invisible format character could hide part of what
//! is being approved. The sanitiser and the length bound are the shell's own
//! (`agent::tools::shell_policy`), so this panel and every other surface that
//! puts such a call to a human show the same text. A target longer than the
//! prompt shows is refused outright — the operator never answers for text it
//! cannot read in full.

use super::{CodingMessage, ask_operator};
use async_trait::async_trait;
use saya_agent::CancellationToken;
use serde_json::Value;
use std::sync::mpsc;

use crate::agent::tools::consent::Operator;
use crate::agent::tools::shell_policy::sanitise_prompt_text;

/// The longest command, working directory or URL this prompt shows, in
/// characters. A longer one is refused before the operator is asked: the cut
/// tail of a command could carry anything, and the operator would be approving
/// text it never saw. The panel's own bound — its prompt is a fixed shape, so
/// it must refuse rather than paint an unbounded target.
const MAX_PROMPT_CHARS: usize = 2048;

/// The operator's approver for an egressing call in the coding panel: the same
/// panel, sender, token and job id the job approval uses. The subject line is
/// the caller's — the root's names its job, a helper's names the helper — and
/// the target itself is what the operator really answers for.
pub(super) struct PanelEgressApproval {
    pub(super) sender: mpsc::SyncSender<CodingMessage>,
    pub(super) token: CancellationToken,
    pub(super) id: String,
}

/// The line that states what is being approved, or `None` when the arguments
/// carry no target this panel can show — which is a refusal, never a prompt
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
impl Operator for PanelEgressApproval {
    async fn ask(&self, subject: &str, arguments: &Value) -> bool {
        // Never ask about a target the operator cannot read in full, and never
        // ask with no target at all: a refusal is the safe answer, and the
        // model is free to send a shorter one.
        let Some(target) = target(arguments) else {
            return false;
        };
        let text = format!(
            "{}\n{}\nUse `/approve {}` or `/deny {}`.",
            subject, target, self.id, self.id
        );
        ask_operator(&self.sender, &self.token, text).await
    }
}
