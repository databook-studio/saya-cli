//! Snapshot tests for the saved-investigation picker overlay: three items
//! with a filter, and the no-match state. Drawn through the real `ui::draw`.

use super::display_row;
use crate::interactive::tui::types::{InvestigationEntry, InvestigationPicker};
use crate::interactive::tui::ui_snapshot_tests::{empty_app, fixed_status, render_buffer};

fn entry(id: &str, name: &str) -> InvestigationEntry {
    InvestigationEntry {
        id: id.to_owned(),
        name: name.to_owned(),
        label: format!("{:<10}  {name}  ·  sqlite  ·  warehouse", "just now"),
    }
}

/// Rows are sanitized (control characters become spaces) and truncated to
/// the overlay's inner width.
#[test]
fn display_row_sanitises_and_truncates() {
    assert_eq!(display_row("ab\u{7}cd", 10), "ab cd");
    assert_eq!(display_row("abcdefghij", 4), "abcd");
    assert_eq!(display_row("héllo", 3), "hél");
}

/// The overlay renders with three items and a filter: the filtered rows are
/// the only ones visible, the filter line is shown, and the filtered-out
/// row is gone.
#[test]
fn overlay_snapshot_three_items_with_filter() {
    let mut app = empty_app();
    let entries = vec![
        entry("recent-orders-abcdef01", "Recent orders"),
        entry("cost-by-region-deadbeef", "Cost by region"),
        entry("weekend-traffic-0123abcd", "Weekend traffic"),
    ];
    let mut picker = InvestigationPicker {
        entries,
        selected: 0,
        capped: false,
        query: String::new(),
    };
    picker.query = "re".into();
    picker.selected = 1;
    app.overlays.investigations = Some(picker);
    let buffer = render_buffer(&app, &fixed_status(), 80, 24);
    assert!(buffer.contains("saved investigations"), "{buffer}");
    assert!(buffer.contains("filter: re"), "{buffer}");
    assert!(buffer.contains("Recent orders"), "{buffer}");
    assert!(buffer.contains("Cost by region"), "{buffer}");
    assert!(!buffer.contains("Weekend traffic"), "{buffer}");
}

/// A filter with no matches says so inside the overlay.
#[test]
fn overlay_with_no_matches_says_so() {
    let mut app = empty_app();
    let entries = vec![entry("recent-orders-abcdef01", "Recent orders")];
    let picker = InvestigationPicker {
        entries,
        selected: 0,
        capped: true,
        query: "zzz".into(),
    };
    app.overlays.investigations = Some(picker);
    let buffer = render_buffer(&app, &fixed_status(), 80, 24);
    assert!(buffer.contains("no saved investigations match"), "{buffer}");
    assert!(!buffer.contains("Recent orders"), "{buffer}");
}
