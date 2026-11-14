//! Text shaping for `config doctor --task` readiness: one summary line, then
//! one line per check. JSON/NDJSON fall out of the serde derive.

use super::{Rendered, TerminalEvent};
use crate::config::doctor_task::CheckStatus;

/// `task <name>: ready` / `task <name>: NOT ready`, then one line per check:
/// `✓ <name>: <detail>` / `! <name>: <detail>` / `✗ <name>: <detail>`, and a
/// check that carries an edit hint adds `   fix: <hint>` under its line.
pub(super) fn task_text(event: &TerminalEvent) -> Rendered {
    // The only caller is the `DoctorTask` arm of `text_event`.
    let TerminalEvent::DoctorTask {
        task,
        ready,
        checks,
    } = event
    else {
        unreachable!("task_text shapes the DoctorTask event")
    };
    let mut stdout = format!(
        "task {task}: {}\n",
        if *ready { "ready" } else { "NOT ready" }
    );
    for check in checks {
        let mark = match check.status {
            CheckStatus::Ok => '✓',
            CheckStatus::Warn => '!',
            CheckStatus::Fail => '✗',
        };
        stdout.push_str(&format!("{mark} {}: {}\n", check.name, check.detail));
        if let Some(hint) = &check.hint {
            stdout.push_str(&format!("   fix: {hint}\n"));
        }
    }
    Rendered {
        stdout,
        stderr: String::new(),
    }
}
