//! The calm status row, its toast neighbour, and the launch caveat's new home.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::model::Model;
use super::msg::Msg;
use super::run_draw::{Facts, frame};
use super::update::update;
use crate::chat::Question;

/// The screen `model` paints at `width × height`.
fn painted(model: &Model, facts: &Facts, width: u16, height: u16) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("the test backend");
    terminal
        .draw(|area| frame(model, facts, area))
        .expect("the frame is painted");
    format!("{}", terminal.backend())
        .lines()
        .map(|row| row.trim_matches('"').to_owned())
        .collect()
}

/// A session on the demo database, launched over `~/shop`.
fn session() -> Model {
    Model {
        database: Some(String::from("demo")),
        ..Model::default()
    }
}

/// The row the status line occupies.
fn status(rows: &[String]) -> &String {
    rows.iter()
        .find(|row| row.contains("? help") || row.contains("working") || row.contains("queued 1"))
        .unwrap_or_else(|| panic!("no status row in {rows:#?}"))
}

#[test]
fn the_status_row_is_the_mode_the_model_and_the_database() {
    let facts = Facts {
        workspace: Some(String::from("~/shop")),
        model: String::from("deepseek/deepseek-v4.1-flash"),
        host_unsandboxed: true,
        sharing: false,
        ..Facts::default()
    };
    for (width, height) in [(100, 30), (60, 24)] {
        let rows = painted(&session(), &facts, width, height);
        let row = status(&rows);
        assert!(
            row.contains("ask │ deepseek-v4.1-flash │ demo"),
            "{width}x{height}: mode, short model, database: {row:?}"
        );
        for gone in [
            "host unsandboxed",
            "no workspace",
            "deepseek-v4.1-flash/flash",
        ] {
            assert!(
                !row.contains(gone),
                "{width}x{height}: {gone:?} is not on the row: {row:?}"
            );
        }
        assert!(
            !row.contains("deepseek/"),
            "{width}x{height}: the short model has no provider prefix: {row:?}"
        );
    }
}

#[test]
fn a_toast_never_touches_the_status_row() {
    let facts = Facts {
        model: String::from("deepseek-v4.1-flash"),
        ..Facts::default()
    };
    let quiet = painted(&session(), &facts, 60, 24);
    let quiet_status = status(&quiet).clone();

    let mut toasting = session();
    toasting.say("mode → plan: write tools refuse until you leave plan");
    let rows = painted(&toasting, &facts, 60, 24);
    let row = status(&rows);

    assert_eq!(
        row, &quiet_status,
        "a toast never rewrites or truncates the status row"
    );
    assert!(
        !row.contains("mode → plan"),
        "the toast is not on the status row: {row:?}"
    );
    let toast = rows
        .iter()
        .position(|line| line.contains("mode → plan"))
        .expect("the toast paints");
    let composer = rows
        .iter()
        .position(|line| line.contains("╭ Message"))
        .expect("the composer paints");
    let status_row = rows
        .iter()
        .position(|line| line == &quiet_status)
        .expect("the status row paints");
    assert_eq!(
        toast + 1,
        composer,
        "the toast is a one-line row directly above the composer: {rows:#?}"
    );
    assert!(
        status_row < toast,
        "the toast sits under the status row: {rows:#?}"
    );
}

#[test]
fn an_unsandboxed_launch_warns_on_the_card_and_in_status_not_on_the_row() {
    let mut model = session();
    model.show_card(Question {
        id: 1,
        caller: String::from("the shell tool"),
        facts: String::from("cargo test"),
        grant: Some(String::from("shell:cargo")),
    });
    let facts = Facts {
        model: String::from("deepseek-v4.1-flash"),
        host_unsandboxed: true,
        ..Facts::default()
    };
    let rows = painted(&model, &facts, 100, 30);

    assert!(
        !status(&rows).contains("unsandboxed"),
        "the status row carries no caveat: {:?}",
        status(&rows)
    );
    assert!(
        rows.iter()
            .any(|row| row.contains("host commands run unsandboxed")),
        "the approval card says the lane is unsandboxed: {rows:#?}"
    );
}

#[test]
fn shift_tab_keeps_the_welcome_screen_and_the_model_name() {
    let mut model = Model::default();
    let cmds = update(
        &mut model,
        Msg::Key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT)),
    );

    assert_eq!(cmds.len(), 1, "the mode change is stated once: {cmds:?}");
    assert!(
        model.turns().is_empty(),
        "a mode change is not a conversation: {:?}",
        model.turns()
    );
    assert!(
        model.toast().is_some(),
        "the mode change is said in the toast row"
    );

    let facts = Facts {
        model: String::from("deepseek/deepseek-v4.1-flash"),
        workspace: Some(String::from("~/shop")),
        ..Facts::default()
    };
    let rows = painted(&model, &facts, 100, 30);
    assert!(
        rows.iter()
            .any(|row| row.contains("Ask about") || row.contains("/connect")),
        "the welcome screen survives Shift+Tab: {rows:#?}"
    );
    assert!(
        status(&rows).contains("deepseek-v4.1-flash"),
        "the model name never leaves the row: {:?}",
        status(&rows)
    );
}
