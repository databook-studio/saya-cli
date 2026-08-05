use super::{
    session_commands::SessionAction, session_resume::block_on, session_state::SessionState,
};
use crate::render::{RenderFormat, TerminalEvent, render_event};
use saya_store::{FsSessionStore, SessionStore};

pub(crate) fn emit_action(
    action: SessionAction,
    format: RenderFormat,
    state: &mut SessionState,
    store: &FsSessionStore,
) -> Result<(), Box<dyn std::error::Error>> {
    match action {
        SessionAction::Message(message) => {
            if message != "Conversation context cleared." {
                state.record("system", &message);
            }
            emit(TerminalEvent::Result { message }, format);
        }
        SessionAction::Agent(_)
        | SessionAction::Schema(_)
        | SessionAction::Sql(_)
        | SessionAction::Resume(_)
        | SessionAction::Exit => {}
        SessionAction::Cancelled => emit(
            TerminalEvent::Diagnostic {
                message: "Request cancelled.".into(),
            },
            format,
        ),
        SessionAction::NotImplemented(feature) => {
            emit(TerminalEvent::NotImplemented { feature }, format)
        }
        SessionAction::Error(message) => emit(TerminalEvent::Error { message }, format),
        SessionAction::History => history(format, state, store)?,
    }
    Ok(())
}

fn history(
    format: RenderFormat,
    state: &mut SessionState,
    store: &FsSessionStore,
) -> Result<(), Box<dyn std::error::Error>> {
    let entries = block_on(store.history())?;
    let message = if entries.is_empty() {
        "No saved sessions.".into()
    } else {
        entries
            .into_iter()
            .map(|entry| format!("{}\t{}", entry.id, entry.modified_unix_ms))
            .collect::<Vec<_>>()
            .join("\n")
    };
    emit(
        TerminalEvent::Result {
            message: message.clone(),
        },
        format,
    );
    state.record("system", message);
    Ok(())
}

fn emit(event: TerminalEvent, format: RenderFormat) {
    let rendered = render_event(&event, format);
    print!("{}", rendered.stdout);
    eprint!("{}", rendered.stderr);
}
