//! Tests for the saved-investigation picker's behaviour: the pure filter,
//! the bounded paginated load, the key actions (Enter shows, `r` replays in
//! the background, Esc closes), and the empty-store state.

use super::{filter_entries, load_investigation_entries};
use crate::cli::InvestigationCommand;
use crate::interactive::tui::application::tests_support::idle_app;
use crate::interactive::tui::keys::handle_key;
use crate::interactive::tui::replay_task::ReplayTask;
use crate::interactive::tui::types::{InvestigationEntry, InvestigationPicker};
use crate::render::RenderFormat;
use ratatui::crossterm::event::{KeyCode, KeyModifiers};
use saya_store::{InvestigationRepository, SqliteStateStore};
use saya_types::SqlDialect;
use saya_types::investigation::{
    INVESTIGATION_FORMAT, INVESTIGATION_FORMAT_VERSION, InvestigationDefinitionV1, InvestigationId,
};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn entry(id: &str, name: &str) -> InvestigationEntry {
    InvestigationEntry {
        id: id.to_owned(),
        name: name.to_owned(),
        label: format!("{:<10}  {name}  ·  sqlite  ·  warehouse", "just now"),
    }
}

fn picker(entries: Vec<InvestigationEntry>) -> InvestigationPicker {
    InvestigationPicker {
        entries,
        selected: 0,
        capped: false,
        query: String::new(),
    }
}

fn definition(id: &str, revision: u32) -> InvestigationDefinitionV1 {
    InvestigationDefinitionV1 {
        format: INVESTIGATION_FORMAT.to_owned(),
        version: INVESTIGATION_FORMAT_VERSION,
        id: InvestigationId::parse(id).unwrap(),
        revision,
        name: "Recent orders".to_owned(),
        description: None,
        sql: format!("select {revision}"),
        parameters: Vec::new(),
        dialect: SqlDialect::Sqlite,
        connection: "warehouse".to_owned(),
        objects: Vec::new(),
        schema_fingerprint: None,
        created_unix_ms: 1_000,
        updated_unix_ms: 1_000,
    }
}

fn three_entries() -> Vec<InvestigationEntry> {
    vec![
        entry("recent-orders-abcdef01", "Recent orders"),
        entry("cost-by-region-deadbeef", "Cost by region"),
        entry("weekend-traffic-0123abcd", "Weekend traffic"),
    ]
}

/// The filter is a case-insensitive substring match over the id and the
/// display name; the empty query keeps every entry.
#[test]
fn filter_matches_id_and_name_case_insensitively() {
    let entries = three_entries();
    let visible = filter_entries(&entries, "re");
    assert_eq!(visible.len(), 2, "recent-orders and cost-by-region match");
    assert_eq!(filter_entries(&entries, "RECENT").len(), 1);
    assert_eq!(
        filter_entries(&entries, "REGION").first().unwrap().id,
        "cost-by-region-deadbeef"
    );
    assert_eq!(filter_entries(&entries, "").len(), 3);
    assert!(filter_entries(&entries, "zzz").is_empty());
}

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
            },
            format: RenderFormat::Text,
            state_db: SqliteStateStore::new(PathBuf::new()),
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

/// Opening with zero saved investigations never opens an overlay: the
/// remedy is said instead.
#[test]
fn opening_with_no_saved_investigations_says_the_remedy() {
    let mut app = idle_app();
    app.open_investigation_picker();
    for _ in 0..200 {
        app.poll_investigation_picker();
        if app.overlays.investigations_loading.is_none() {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        app.overlays.investigations_loading.is_none(),
        "the load completed"
    );
    assert!(
        app.overlays.investigations.is_none(),
        "no overlay for an empty store"
    );
    let block = app
        .transcript
        .blocks()
        .last()
        .expect("the empty state is said");
    assert!(
        block
            .text
            .contains("No saved investigations — save one with /investigation save <name>"),
        "the remedy is named: {:?}",
        block.text
    );
}

/// More than one page of 50 is pulled through pagination, in id order —
/// the load never stops at the first page, and the picker cap (500, the
/// collection cap the store itself enforces on create) bounds it.
#[test]
fn load_pulls_more_than_one_page_in_id_order() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "saya-investigation-picker-pages-{}-{nanos}",
        std::process::id()
    ));
    let repo = InvestigationRepository::new(root.clone());
    for i in 0..60 {
        repo.create(&definition(&format!("doc{i:03}-aaaaaaaa"), 1))
            .unwrap();
    }
    let (entries, capped) = load_investigation_entries(&root).unwrap();
    assert_eq!(
        entries.len(),
        60,
        "pagination continues past the first page of 50"
    );
    assert!(!capped, "the collection cap was not reached");
    let ids: Vec<&str> = entries.iter().map(|e| e.id.as_str()).collect();
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    assert_eq!(ids, sorted, "entries stay in id order");
    let _ = std::fs::remove_dir_all(&root);
}
