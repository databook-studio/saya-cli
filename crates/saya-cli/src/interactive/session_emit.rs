use super::{
    session_commands::SessionAction, session_resume::block_on, session_state::SessionState,
};
use crate::render::{RenderFormat, TerminalEvent, render_event};
use saya_store::{
    FsSessionStore, MAX_SESSION_HISTORY_PAGE_SIZE, SessionHistoryQuery, SessionStore,
};

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
        // Doctor is intercepted in the session loop (it needs `runtime`) and
        // never reaches here; the arm keeps the match exhaustive.
        SessionAction::Doctor => {}
        SessionAction::Compact
        | SessionAction::Agent(_)
        | SessionAction::Schema(_)
        | SessionAction::Sql(_)
        | SessionAction::Contracts(_)
        | SessionAction::Resume(_)
        // The `/run` family is intercepted in the session loop (it needs the
        // runtime, and the nested run's output never passes through this
        // seam); the arms keep the match exhaustive. `/allow` and `/grants`
        // are intercepted too — they seed and read the session's grant
        // store, which lives in the runtime.
        | SessionAction::Allow(_)
        | SessionAction::Grants
        | SessionAction::Run(_)
        | SessionAction::RunCancel(_)
        | SessionAction::Runs(_)
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
        SessionAction::Export(_) => emit(
            TerminalEvent::Error {
                message: "export is only available in the interactive TUI".into(),
            },
            format,
        ),
        SessionAction::Chart(_) => emit(
            TerminalEvent::Error {
                message: "chart is only available in the interactive TUI".into(),
            },
            format,
        ),
        SessionAction::Explain(_) => emit(
            TerminalEvent::Error {
                message: "explain is only available in the interactive TUI".into(),
            },
            format,
        ),
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
    let page = block_on(
        store.history(
            SessionHistoryQuery::first_page(MAX_SESSION_HISTORY_PAGE_SIZE)
                .expect("history page bound is valid"),
        ),
    )?;
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_millis())
        .unwrap_or(0);
    let message = if page.entries.is_empty() {
        "No saved sessions.".into()
    } else {
        let more = page.next_cursor.is_some();
        let mut message = page
            .entries
            .into_iter()
            .map(|entry| {
                let age = super::tui::replay::relative_time(
                    now_ms.saturating_sub(entry.modified_unix_ms),
                );
                format!("{}\t{}", entry.id, age)
            })
            .collect::<Vec<_>>()
            .join("\n");
        if more {
            message.push_str("\n(more saved sessions available)");
        }
        message
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
