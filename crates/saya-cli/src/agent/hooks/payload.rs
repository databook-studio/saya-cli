//! The bounded JSON description one hook reads on stdin.
//!
//! The payload is built here, once, and every field is bounded before it is
//! serialized: a hook is a user command, but the *arguments* it is shown come
//! from the model, so they are redacted exactly as tool-call evidence is
//! ([`saya_types::redact`]) and cut to a named budget with an elision marker
//! between the retained head and tail — the same shape
//! [`saya_agent::bound_tool_transcript`] gives a recorded call.
//!
//! `tool_input` is a **string**, not an object: redaction rewrites text, and a
//! redacted JSON document is not necessarily parseable JSON. Handing a hook a
//! string it can read (and `jq -r` or a shell pattern-match) is honest; handing
//! it an object that sometimes fails to parse is not.

use saya_agent::bound_tool_transcript;
use saya_config::HookEvent;
use serde_json::{Value, json};

use super::HookCaller;

/// The whole payload's byte ceiling — the harness shell's own stdin bound, so
/// a payload this module produces can never be refused at the spawn.
pub(super) const MAX_INPUT_BYTES: usize = saya_harness::runner::shell::MAX_STDIN_BYTES;

/// The redacted arguments' own budget before the whole-payload fit loop starts
/// shrinking it.
const MAX_ARGUMENT_BYTES: usize = 4 * 1024;

/// The smallest argument budget the fit loop tries before it drops the
/// arguments entirely. A payload bounded to this fits [`MAX_INPUT_BYTES`]
/// whatever the escaping.
const MIN_ARGUMENT_BYTES: usize = 0;

/// The tool name's bound. A name is an identifier, not content; the bound is
/// here so no field is unbounded.
const MAX_TOOL_NAME_BYTES: usize = 128;

/// The caller's id bound, matching the recorded tool-call id.
const MAX_CALLER_ID_BYTES: usize = 128;

/// The description of a tool call, for the two tool events. `succeeded` is
/// present only for `PostToolUse`, where the call's outcome is already known.
pub(super) fn tool_event(
    caller: &HookCaller,
    event: HookEvent,
    tool: &str,
    arguments: &Value,
    succeeded: Option<bool>,
) -> String {
    let redacted = saya_types::redact(&serde_json::to_string(arguments).unwrap_or_default());
    let mut budget = MAX_ARGUMENT_BYTES;
    loop {
        let mut payload = json!({
            "hook_event_name": event.wire_name(),
            "tool_name": bound_tool_transcript(tool, MAX_TOOL_NAME_BYTES),
            "tool_input": bound_tool_transcript(&redacted, budget),
        });
        if let Some(succeeded) = succeeded {
            payload["tool_succeeded"] = Value::Bool(succeeded);
        }
        caller.decorate(&mut payload);
        let text = payload.to_string();
        if text.len() <= MAX_INPUT_BYTES || budget == MIN_ARGUMENT_BYTES {
            return text;
        }
        budget /= 2;
    }
}

/// The description of a permission request: the tool and its redacted,
/// bounded arguments, exactly as a tool event carries them. Exit `2` from a
/// hook reading this denies the call; exit `0` falls through to the ask.
pub(super) fn permission_event(caller: &HookCaller, tool: &str, arguments: &Value) -> String {
    tool_event(caller, HookEvent::PermissionRequest, tool, arguments, None)
}

/// The description of a session boundary, for `SessionStart` and
/// `SessionEnd`: the event and who it belongs to, and nothing else — there is
/// no call to describe.
pub(super) fn lifecycle_event(caller: &HookCaller, event: HookEvent) -> String {
    let mut payload = json!({"hook_event_name": event.wire_name()});
    caller.decorate(&mut payload);
    payload.to_string()
}

/// The description of a turn ending, for the `stop` event: the event and who
/// finished, and nothing else — there is no call to describe.
pub(super) fn stop_event(caller: &HookCaller) -> String {
    let mut payload = json!({"hook_event_name": HookEvent::Stop.wire_name()});
    caller.decorate(&mut payload);
    payload.to_string()
}

/// One caller id, bounded: an id is an identifier, and no field is unbounded.
fn bounded_id(id: &str) -> Value {
    Value::String(bound_tool_transcript(id, MAX_CALLER_ID_BYTES))
}

impl HookCaller {
    /// Adds this caller's identity to a payload: `job_id` for a coding job,
    /// `run_id` for a `saya run`, `session_id` for an interactive session. A
    /// helper adds both its job and its own agent id, so a script can block a
    /// job's helpers without blocking the job's root. A caller that owns no
    /// record — the one-shot `ask` path, or a session universe with no
    /// acquisition behind it — adds no id field at all rather than a made-up
    /// one.
    fn decorate(&self, payload: &mut Value) {
        match self {
            HookCaller::Job(id) => payload["job_id"] = bounded_id(id),
            HookCaller::Helper { agent_id, job_id } => {
                payload["job_id"] = bounded_id(job_id);
                payload["agent_id"] = bounded_id(agent_id);
            }
            HookCaller::Run(id) => payload["run_id"] = bounded_id(id),
            HookCaller::Session(Some(id)) => payload["session_id"] = bounded_id(id),
            HookCaller::Session(None) | HookCaller::Ask => {}
        }
    }
}
