//! Wide-table view actions: horizontal scrolling, pinning the first column,
//! and `/columns` selection. Each mutates only the view state on `App`; the
//! transcript block text (the full table) is untouched, so copy and persistence
//! still see every column.

use super::super::table;
use super::super::transcript::BlockKind;
use super::super::types::App;

impl App {
    /// Scrolls the wide-table view one column left.
    pub(crate) fn scroll_table_left(&mut self) {
        self.wide_table.h_offset = self.wide_table.h_offset.saturating_sub(1);
    }

    /// Scrolls the wide-table view one column right, clamped to the last
    /// result table's column count so the offset cannot run past the data.
    pub(crate) fn scroll_table_right(&mut self) {
        let max = self
            .last_table_column_count()
            .saturating_sub(1)
            .max(self.wide_table.h_offset);
        self.wide_table.h_offset = (self.wide_table.h_offset + 1).min(max);
    }

    /// Toggles whether the first column stays pinned while the rest scroll.
    pub(crate) fn toggle_pin_first_column(&mut self) {
        self.wide_table.pin_first = !self.wide_table.pin_first;
        let state = if self.wide_table.pin_first {
            "on"
        } else {
            "off"
        };
        self.transcript.push(
            BlockKind::System,
            format!("Pin first column {state} — Ctrl+, / Ctrl+. to scroll, Ctrl+P to toggle."),
        );
    }

    /// Applies `/columns [list]`. With no argument (or `all`/`off`) every
    /// column is shown and the available names are listed; otherwise the view
    /// is restricted to the named columns. The horizontal offset resets so the
    /// new selection is visible from its first column.
    pub(crate) fn set_visible_columns(&mut self, arg: Option<String>) {
        let trimmed = arg.unwrap_or_default();
        let reset = trimmed.trim().is_empty()
            || trimmed.trim().eq_ignore_ascii_case("all")
            || trimmed.trim().eq_ignore_ascii_case("off");
        if reset {
            self.wide_table.columns = None;
            self.wide_table.h_offset = 0;
            let names = self.last_table_column_names();
            let message = match names {
                Some(names) if !names.is_empty() => {
                    format!("Showing all columns: {}", names.join(", "))
                }
                _ => "Showing all columns.".to_string(),
            };
            self.transcript.push(BlockKind::System, message);
            return;
        }

        let selected: Vec<String> = trimmed
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if selected.is_empty() {
            self.wide_table.columns = None;
        } else {
            self.wide_table.columns = Some(selected);
        }
        self.wide_table.h_offset = 0;
        let message = match &self.wide_table.columns {
            Some(names) => format!("Showing columns: {}", names.join(", ")),
            None => "Showing all columns.".to_string(),
        };
        self.transcript.push(BlockKind::System, message);
    }

    fn last_table_column_count(&self) -> usize {
        self.last_table_column_names()
            .map(|names| names.len())
            .unwrap_or(0)
    }

    fn last_table_column_names(&self) -> Option<Vec<String>> {
        let block = self
            .transcript
            .blocks()
            .iter()
            .rev()
            .find(|block| block.kind == BlockKind::Table)?;
        table::column_names(&block.text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interactive::tui::application::tests_support::idle_app;
    use crate::interactive::tui::keys::handle_key;
    use crate::interactive::tui::table::format_table;
    use crate::interactive::tui::transcript::BlockKind;
    use ratatui::crossterm::event::{KeyCode, KeyModifiers};
    use saya_types::QueryResult;

    fn push_table(app: &mut App, result: &QueryResult) {
        app.transcript.push(BlockKind::Table, format_table(result));
    }

    fn wide_result() -> QueryResult {
        QueryResult {
            columns: vec!["id".into(), "name".into(), "status".into()],
            rows: vec![serde_json::json!([1, "alice", "ok"])],
            row_count: 1,
            truncated: false,
            executed_sql: "SELECT * FROM t".into(),
        }
    }

    #[test]
    fn scroll_left_and_right_move_the_offset() {
        let mut app = idle_app();
        app.wide_table.h_offset = 3;
        app.scroll_table_left();
        assert_eq!(app.wide_table.h_offset, 2);
        app.scroll_table_left();
        app.scroll_table_left();
        app.scroll_table_left();
        assert_eq!(app.wide_table.h_offset, 0, "clamps at zero");
    }

    #[test]
    fn scroll_right_clamps_to_the_last_table_column_count() {
        let mut app = idle_app();
        push_table(&mut app, &wide_result()); // 3 columns
        app.scroll_table_right();
        app.scroll_table_right();
        app.scroll_table_right();
        app.scroll_table_right();
        assert_eq!(
            app.wide_table.h_offset, 2,
            "cannot scroll past the last column"
        );
    }

    #[test]
    fn toggle_pin_flips_state_and_announces_it() {
        let mut app = idle_app();
        assert!(!app.wide_table.pin_first);
        app.toggle_pin_first_column();
        assert!(app.wide_table.pin_first);
        let notice = app.transcript.blocks().last().expect("a notice was pushed");
        assert!(notice.text.contains("on"), "{}", notice.text);
    }

    #[test]
    fn columns_command_filters_by_name_and_resets_offset() {
        let mut app = idle_app();
        push_table(&mut app, &wide_result());
        app.wide_table.h_offset = 2;
        app.set_visible_columns(Some("name, status".into()));
        assert_eq!(
            app.wide_table.columns,
            Some(vec!["name".to_string(), "status".to_string()])
        );
        assert_eq!(app.wide_table.h_offset, 0, "offset resets on selection");
    }

    #[test]
    fn columns_command_with_no_arg_lists_available_columns() {
        let mut app = idle_app();
        push_table(&mut app, &wide_result());
        app.set_visible_columns(None);
        assert!(app.wide_table.columns.is_none());
        let notice = app.transcript.blocks().last().expect("a notice was pushed");
        assert!(
            notice.text.contains("id") && notice.text.contains("name"),
            "lists the available columns: {}",
            notice.text
        );
    }

    #[test]
    fn columns_all_resets_a_prior_selection() {
        let mut app = idle_app();
        push_table(&mut app, &wide_result());
        app.set_visible_columns(Some("id".into()));
        app.set_visible_columns(Some("all".into()));
        assert!(app.wide_table.columns.is_none());
    }

    /// The keybindings route to the view actions. Ctrl+,/Ctrl+P are global and
    /// never reach the input editor (comma/period only act as chords).
    #[test]
    fn keybindings_drive_the_wide_table_view() {
        let mut app = idle_app();
        push_table(&mut app, &wide_result());

        app.wide_table.h_offset = 2;
        handle_key(&mut app, KeyCode::Char(','), KeyModifiers::CONTROL);
        assert_eq!(app.wide_table.h_offset, 1, "Ctrl+, scrolls left");

        handle_key(&mut app, KeyCode::Char('.'), KeyModifiers::CONTROL);
        assert_eq!(app.wide_table.h_offset, 2, "Ctrl+. scrolls right");

        assert!(!app.wide_table.pin_first);
        handle_key(&mut app, KeyCode::Char('p'), KeyModifiers::CONTROL);
        assert!(app.wide_table.pin_first, "Ctrl+P toggles pin");
    }
}
