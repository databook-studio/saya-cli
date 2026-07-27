use crate::render::{RenderFormat, TerminalEvent, render_event};
use saya_types::ConnectionError;

pub(super) fn emit(event: TerminalEvent, format: RenderFormat) {
    let rendered = render_event(&event, format);
    print!("{}", rendered.stdout);
    eprint!("{}", rendered.stderr);
}

pub(super) fn result(
    message: String,
    format: RenderFormat,
) -> Result<i32, Box<dyn std::error::Error>> {
    emit(TerminalEvent::Result { message }, format);
    Ok(0)
}

pub(super) fn unavailable(
    code: i32,
    feature: String,
    format: RenderFormat,
) -> Result<i32, Box<dyn std::error::Error>> {
    emit(TerminalEvent::NotImplemented { feature }, format);
    Ok(code)
}

pub(super) fn failure(
    code: i32,
    error: ConnectionError,
    format: RenderFormat,
) -> Result<i32, Box<dyn std::error::Error>> {
    emit(
        TerminalEvent::Error {
            message: error.to_string(),
        },
        format,
    );
    Ok(code)
}
