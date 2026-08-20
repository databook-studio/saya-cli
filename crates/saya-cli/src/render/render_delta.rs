use super::{Rendered, sanitize_terminal};

pub(super) fn text(text: &str) -> Rendered {
    Rendered {
        stdout: sanitize_terminal(text),
        stderr: String::new(),
    }
}
