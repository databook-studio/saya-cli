use super::{Rendered, TerminalEvent};

pub(super) fn render(event: &TerminalEvent) -> Rendered {
    let line = serde_json::to_string(event).expect("terminal event is serializable") + "\n";
    match event {
        TerminalEvent::Diagnostic { .. } | TerminalEvent::Error { .. } => Rendered {
            stdout: String::new(),
            stderr: line,
        },
        _ => Rendered {
            stdout: line,
            stderr: String::new(),
        },
    }
}
