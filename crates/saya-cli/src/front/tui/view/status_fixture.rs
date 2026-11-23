//! The facts the status-line tests paint, and the readings they take of a
//! rendered row.

use ratatui::text::Line;
use saya_agent::PermissionMode;

use super::status::{ActivityState, StatusFacts};
use super::status_fit::GAP;
use crate::interactive::tui::wrap::cell_width;

/// The row's everyday facts: the mode, a provider-prefixed model, two
/// databases, the sharing caveat, and a turn six seconds in. The unsandboxed
/// warning is not here: it belongs to the approval card.
pub(crate) fn full() -> StatusFacts {
    StatusFacts {
        mode: PermissionMode::Ask,
        databases: vec!["demo".to_owned(), "shop".to_owned()],
        model: "deepseek/deepseek-v4.1-flash".to_owned(),
        sharing: true,
        activity: ActivityState::Working { seconds: 6 },
        ..StatusFacts::default()
    }
}

/// A session with nothing attached — the row's one absence.
pub(crate) fn bare() -> StatusFacts {
    StatusFacts {
        model: "qwen3-coder".to_owned(),
        ..StatusFacts::default()
    }
}

/// The row exactly as a terminal would paint it, right-aligned padding and all.
pub(crate) fn words(line: &Line<'_>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect()
}

/// The row without the padding the right alignment adds.
pub(crate) fn row(line: &Line<'_>) -> String {
    words(line).trim_end().to_owned()
}

/// The cells the row occupies.
pub(crate) fn cells(line: &Line<'_>) -> usize {
    cell_width(&words(line))
}

/// Asserts the row is `left`, then air alone, then `right`, filling exactly
/// `width` cells — the shape every row has when it fits.
///
/// The air is checked to be air rather than compared literally: how many cells
/// of it there are is the right alignment's business, but that there is air at
/// all, and that nothing else sits between the two groups, is the row's.
pub(crate) fn assert_row(line: &Line<'_>, width: usize, left: &str, right: &str) {
    let text = row(line);
    assert!(
        text.starts_with(left),
        "the left group should read {left:?}: {text:?}"
    );
    assert!(
        text.ends_with(right),
        "the right side should read {right:?}: {text:?}"
    );
    let air = &text[left.len()..text.len() - right.len()];
    assert!(
        air.len() >= GAP && air.chars().all(|character| character == ' '),
        "only air separates the two groups: {air:?}"
    );
    assert_eq!(
        cells(line),
        width,
        "a right-aligned row fills the width it is given: {text:?}"
    );
}
