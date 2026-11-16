//! The shell's own words in the operator's prompt: the line that names who is
//! asking, paired with the operator that answers. The operator itself — the
//! human, the serialisation that keeps one ask pending at a time, and the
//! caller line's shape — is the shared seam
//! ([`crate::agent::tools::consent`]), because a `web_fetch` call is put to the
//! same person through it.

use std::sync::Arc;

use async_trait::async_trait;
use saya_agent::{ApprovalDecider, ToolDefinition};
use serde_json::Value;

use super::schema::definition;
use crate::agent::tools::consent::{Caller, Operator, Prompt};

/// How the shell names its own call: the imperative a root's or a session's
/// line opens with, and the verb a helper's sentence uses.
const IMPERATIVE: &str = "Run a shell command";
const VERB: &str = "run a shell command";

/// The shell's line for `caller`, in the shell's own words: the text the
/// operator reads above the command. Read by the surfaces that assert the
/// wording, so a test and the prompt cannot state it differently.
pub(crate) fn line(caller: &Caller) -> String {
    caller.line(IMPERATIVE, VERB)
}

/// The shell's prompt: the operator, and the line that names the caller
/// ([`Caller`]) in the shell's own words. A shell with nobody to ask has no
/// prompt at all, and `None` on the host lane under `ask` is the headless case.
pub(crate) fn prompt(operator: Arc<dyn Operator>, caller: Caller) -> Prompt {
    Prompt::new(operator, line(&caller))
}

/// The bridge from the model-facing approval contract to a shell prompt: the
/// interactive session's own surface is an [`ApprovalDecider`], so a host shell
/// call is put to it under the shell's advertised definition. The session
/// words its own prompt from the tool call, so the line the session's caller
/// builds is not rendered by that surface.
pub(crate) struct DeciderShellOperator {
    decider: Arc<dyn ApprovalDecider>,
    definition: ToolDefinition,
}

impl DeciderShellOperator {
    pub(crate) fn new(decider: Arc<dyn ApprovalDecider>) -> Self {
        Self {
            decider,
            definition: definition(),
        }
    }
}

#[async_trait]
impl Operator for DeciderShellOperator {
    async fn ask(&self, _line: &str, arguments: &Value) -> bool {
        self.decider.approve(&self.definition, arguments).await
    }
}
