//! The calm status row, its toast neighbour, and the launch caveat's new home.

use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::model::Model;
use super::msg::Msg;
use super::run_draw::{Facts, frame};
use super::update::update;
use super::view::Meters;
use super::view::cards_fixture::shell_question;

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

/// The meter the settled turn reported paints on the row's left group, and it
/// is the first thing a narrow row gives up: at 60 columns the mode, the model,
/// the database and `? help` all stay.
#[test]
fn the_meter_paints_wide_and_goes_first_narrow() {
    let facts = Facts {
        model: String::from("deepseek/deepseek-v4.1-flash"),
        sharing: true,
        ..Facts::default()
    };
    let mut model = session();
    update(
        &mut model,
        Msg::Meters(Meters {
            input_tokens: Some(51_200),
            window_tokens: Some(128_000),
            cost_micros: Some(31_200),
        }),
    );

    let wide = painted(&model, &facts, 100, 30);
    assert!(
        status(&wide).contains("ask │ deepseek-v4.1-flash │ demo")
            && status(&wide).contains("ctx 40% · $0.0312"),
        "the meter joins the row's left group: {:?}",
        status(&wide)
    );

    let narrow = painted(&model, &facts, 60, 24);
    let row = status(&narrow);
    assert!(
        row.contains("ask │ deepseek-v4.1-flash │ demo") && row.contains("? help"),
        "mode, model, database and help stay: {row:?}"
    );
    assert!(
        !row.contains("ctx") && !row.contains('$'),
        "the meter is dropped first: {row:?}"
    );
}

/// A turn that reported input tokens against no known window leaves the row
/// without a meter: the window is never guessed from the count.
#[test]
fn an_unknown_window_paints_no_meter() {
    let facts = Facts {
        model: String::from("gpt-4o"),
        ..Facts::default()
    };
    let mut model = session();
    update(
        &mut model,
        Msg::Meters(Meters {
            input_tokens: Some(51_200),
            ..Meters::default()
        }),
    );

    let rows = painted(&model, &facts, 100, 30);
    assert!(
        !status(&rows).contains("ctx") && !status(&rows).contains('$'),
        "nothing reported, nothing shown: {:?}",
        status(&rows)
    );
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
fn an_unsandboxed_launch_warns_on_the_card_and_never_on_the_row() {
    let mut model = session();
    // A shell's call: the one family whose ask is answered on the host lane, so
    // the one whose place line states the lane.
    model.show_card(shell_question());
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
            .any(|row| row.contains("on your machine, not sandboxed")),
        "the approval card's place line says the lane is unsandboxed: {rows:#?}"
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
