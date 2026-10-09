//! Key actions on the open picker: Enter shows the selected investigation
//! through the same adapter `/investigation show <id>` uses, `r` starts the
//! same background replay `/investigation run <id>` does (refusing while a
//! replay is already running), and typing, navigation, Esc, and `r` route
//! through the real `handle_key`.

use super::{definition, picker, three_entries};
use crate::cli::InvestigationCommand;
use crate::interactive::tui::application::tests_support::idle_app;
use crate::interactive::tui::keys::handle_key;
use crate::interactive::tui::replay_task::ReplayTask;
use crate::render::RenderFormat;
use ratatui::crossterm::event::{KeyCode, KeyModifiers};
use saya_store::{InvestigationRepository, SqliteStateStore};
use std::path::PathBuf;

/// Enter shows the selected investigation through the same adapter
/// `/investigation show <id>` uses: the transcript block is the
/// definition's JSON plus the binding line, produced by the shared command.
#[test]
fn enter_shows_the_investigation_block() {
    let mut app = idle_app();
    let repo = InvestigationRepository::new(app.runtime.investigations_root.clone());
    repo.create(&definition("recent-orders-abcdef01", 3))
        .unwrap();
    app.overlays.investigations = Some(picker(three_entries()));
    app.investigations_confirm();
    assert!(
        app.overlays.investigations.is_none(),
        "Enter closes the picker"
    );
    let block = app.transcript.blocks().last().expect("the show block");
    assert!(
        block.text.contains("recent-orders-abcdef01") && block.text.contains("Recent orders"),
        "the definition is shown: {:?}",
        block.text
    );
    assert!(
        block.text.contains("local binding: none"),
        "the binding line is included: {:?}",
        block.text
    );
}

/// `r` starts the same background replay `/investigation run <id>` does: a
/// replay task is tracked for the loop, spawned without blocking.
#[test]
fn r_starts_the_same_background_replay() {
    let mut app = idle_app();
    app.overlays.investigations = Some(picker(three_entries()));
    app.investigations_run();
    assert!(app.overlays.investigations.is_none(), "r closes the picker");
    let (_, task, _) = app.replay_task.as_ref().expect("the replay is tracked");
    assert_eq!(task.id, "recent-orders-abcdef01");
    assert!(
        matches!(
            task.command,
            InvestigationCommand::Run {
                connection: None,
                revalidate: false,
                ..
            }
        ),
        "the task is the plain run command: {:?}",
        task.command
    );
}

/// `r` while a replay is already running refuses with the same message the
/// command path shows — the running replay is never replaced.
#[test]
fn r_refuses_when_a_replay_is_already_running() {
    let mut app = idle_app();
    let (_, rx) = std::sync::mpsc::channel();
    app.replay_task = Some((
        rx,
        ReplayTask {
            id: "earlier-aaaaaaaa".to_owned(),
            command: InvestigationCommand::Run {
                id: "earlier-aaaaaaaa".to_owned(),
                connection: None,
                revalidate: false,
                report: None,
                rows: None,
                overwrite: false,
                params: Vec::new(),
            },
            format: RenderFormat::Text,
            state_db: SqliteStateStore::new(PathBuf::new()),
            cancellation: saya_agent::CancellationToken::new(),
        },
        std::time::Instant::now(),
    ));
    app.overlays.investigations = Some(picker(three_entries()));
    app.investigations_run();
    let block = app.transcript.blocks().last().expect("the refusal is said");
    assert!(
        block.text.contains("already running"),
        "the busy refusal is said: {:?}",
        block.text
    );
    assert_eq!(
        app.replay_task.as_ref().unwrap().1.id,
        "earlier-aaaaaaaa",
        "the running replay is untouched"
    );
}

/// The key routing through the real `handle_key`: typing filters,
/// navigation moves within the filtered list, Esc closes, `r` runs.
#[test]
fn keys_filter_navigate_close_and_run() {
    let mut app = idle_app();
    app.overlays.investigations = Some(picker(three_entries()));
    handle_key(&mut app, KeyCode::Char('e'), KeyModifiers::NONE);
    assert_eq!(
        app.overlays.investigations.as_ref().unwrap().query,
        "e",
        "typing filters"
    );
    handle_key(&mut app, KeyCode::Down, KeyModifiers::NONE);
    assert_eq!(
        app.overlays.investigations.as_ref().unwrap().selected,
        1,
        "navigation moves within the filtered list"
    );
    handle_key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert!(
        app.overlays.investigations.is_none(),
        "Esc closes the picker"
    );
    // `r` runs through the same routing: reopen, then press `r`.
    app.overlays.investigations = Some(picker(three_entries()));
    handle_key(&mut app, KeyCode::Char('r'), KeyModifiers::NONE);
    assert!(
        app.replay_task.is_some(),
        "r dispatches the background replay"
    );
}
