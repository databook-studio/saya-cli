use super::Rendered;

pub(super) fn text(text: &str) -> Rendered {
    Rendered {
        stdout: text.into(),
        stderr: String::new(),
    }
}
