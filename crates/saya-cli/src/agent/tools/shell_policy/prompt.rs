//! The shell's own words in the operator's prompt: the line that names who is
//! asking, paired with the operator that answers. The operator itself — the
//! human, the serialisation that keeps one ask pending at a time, and the
//! caller line's shape — is the shared seam
//! ([`crate::agent::tools::consent`]), because a `web_fetch` call is put to the
//! same person through it.

use std::sync::Arc;

use saya_agent::SessionGrants;

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

/// The shell's prompt: the operator, the line that names the caller
/// ([`Caller`]) in the shell's own words, and the store the operator remembers
/// a "yes for this run" in. A shell with nobody to ask has no prompt at all,
/// and `None` on the host lane under `ask` is the headless case — which is
/// also the only case with nothing remembered: the store belongs to the
/// operator, so every shell of one job shares the job's.
pub(crate) fn prompt(operator: Arc<dyn Operator>, caller: Caller) -> Prompt {
    let grants: Option<SessionGrants> = operator.grants();
    Prompt::new(operator, line(&caller)).with_grants(grants)
}
