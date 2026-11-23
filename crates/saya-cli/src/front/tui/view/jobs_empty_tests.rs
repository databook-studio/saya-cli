//! The jobs drawer with nothing to list: a one-line notice, not a panel.

use ratatui::Terminal;
use ratatui::backend::TestBackend;

use crate::front::tui::model::Model;
use crate::front::tui::run_draw::{Facts, frame};

/// The screen `model` paints at `width × height`.
fn painted(model: &Model, width: u16, height: u16) -> Vec<String> {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("the test backend");
    terminal
        .draw(|area| frame(model, &Facts::default(), area))
        .expect("the frame is painted");
    format!("{}", terminal.backend())
        .lines()
        .map(|row| row.trim_matches('"').to_owned())
        .collect()
}

#[test]
fn an_empty_drawer_is_one_row_and_not_a_panel() {
    for (width, height) in [(100, 30), (60, 24)] {
        let mut model = Model::default();
        model.open_jobs();
        let rows = painted(&model, width, height);

        let notices: Vec<&String> = rows.iter().filter(|row| row.contains("no jobs")).collect();
        assert_eq!(
            notices.len(),
            1,
            "{width}x{height}: the empty drawer says one line: {rows:#?}"
        );
        assert!(
            !rows.iter().any(|row| row.contains("Background jobs")),
            "{width}x{height}: no panel is drawn for nothing: {rows:#?}"
        );
        assert!(
            rows.iter().any(|row| row.contains("Message")),
            "{width}x{height}: the composer is untouched: {rows:#?}"
        );
        assert!(
            rows.iter().any(|row| row.contains("? help")),
            "{width}x{height}: the status row is untouched: {rows:#?}"
        );
    }
}
