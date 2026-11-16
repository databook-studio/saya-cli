//! `submit_plan`'s fact lines: the plan itself, so the operator approves the
//! text they are reading rather than a tool name.
//!
//! The plan is model-authored, so it goes through the shell's own sanitiser
//! ([`sanitise_prompt_text`](crate::agent::tools::shell_policy::sanitise_prompt_text)):
//! a raw ESC or CR could rewrite the lines being read, and an invisible format
//! character could hide part of what is being approved. Nothing is cut or
//! rewritten — the gate refuses a plan longer than its own bound before anyone
//! is asked, so the operator reads the whole plan, exactly as the gate read it.
//!
//! No session line and no scope sentence: the gate's ask is not a call the
//! grant store answers — no token pre-answers a plan — so there is no history
//! to state and no `[s]` answer to describe.

use serde_json::Value;

use crate::agent::tools::coding::plan_gate::MAX_PLAN_CHARS;
use crate::agent::tools::shell_policy::sanitise_prompt_text;

use super::body;

/// `submit_plan`'s fact lines. `None` for a call that is not one bounded plan
/// the gate refuses that shape without asking anyone, so there is nothing to
/// show.
pub(super) fn facts(arguments: &Value) -> Option<String> {
    let plan = arguments.get("plan").and_then(Value::as_str)?;
    if plan.is_empty() {
        return None;
    }
    Some(body(
        "submit_plan — approves a plan and opens the write surface".to_string(),
        vec![
            format!("  plan: {}", sanitise_prompt_text(plan)),
            format!(
                "  ≤ {MAX_PLAN_CHARS} characters · approving opens the write surface for the \
                 rest of this session; leaving `plan` closes it again"
            ),
        ],
    ))
}
