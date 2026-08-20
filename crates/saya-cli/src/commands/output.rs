use crate::render::{RenderFormat, TerminalEvent, render_event};
use saya_types::ConnectionError;
use std::cell::RefCell;

// A thread-local capture buffer. When set, `emit` appends rendered output here
// instead of writing to the process stdout/stderr, so tests can assert on the
// exact bytes a command would have printed without racing the global file
// descriptors under parallel test runs. Production code never sets it, so the
// real CLI path is unchanged: `emit` prints exactly as before.
thread_local! {
    static CAPTURE: RefCell<Option<(String, String)>> = const { RefCell::new(None) };
}

pub fn emit(event: TerminalEvent, format: RenderFormat) {
    let rendered = render_event(&event, format);
    CAPTURE.with(|cell| match cell.borrow_mut().as_mut() {
        Some((stdout, stderr)) => {
            stdout.push_str(&rendered.stdout);
            stderr.push_str(&rendered.stderr);
        }
        None => {
            print!("{}", rendered.stdout);
            eprint!("{}", rendered.stderr);
        }
    });
}

/// Begins capturing `emit` output on this thread. Pair with
/// [`capture_output_take`]. Nested starts are a programming error and panic so a
/// forgotten `take` cannot silently swallow a later test's output.
pub fn capture_output_start() {
    CAPTURE.with(|cell| {
        let mut slot = cell.borrow_mut();
        assert!(
            slot.is_none(),
            "capture_output_start called without a matching capture_output_take"
        );
        *slot = Some((String::new(), String::new()));
    });
}

/// Returns and clears the (stdout, stderr) captured since
/// [`capture_output_start`]; panics if capture was not started.
pub fn capture_output_take() -> (String, String) {
    CAPTURE.with(|cell| {
        cell.borrow_mut()
            .take()
            .expect("capture_output_take called without a matching capture_output_start")
    })
}

pub fn result(message: String, format: RenderFormat) -> Result<i32, Box<dyn std::error::Error>> {
    emit(TerminalEvent::Result { message }, format);
    Ok(0)
}

pub fn failure(
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

pub fn failure_message(
    code: i32,
    message: String,
    format: RenderFormat,
) -> Result<i32, Box<dyn std::error::Error>> {
    emit(TerminalEvent::Error { message }, format);
    Ok(code)
}
