//! The `@` list's pane: the paths it paints, the row it marks, and the two
//! shapes it must keep — no row wider than the pane, and an ASCII form for
//! every glyph it draws.

use super::composer::Row;
use super::mention::{FINDING, HINT_ASCII, MAX_VISIBLE, NO_MATCH, TITLE, rows};
use crate::front::tui::model::Model;
use crate::front::tui::model_mention::Mention;
use crate::front::tui::theme::Charset;

/// A model with the list open on `paths`, as the walk would have filled it.
fn offering(paths: &[&str]) -> Model {
    let mut model = Model::default();
    model.push('@');
    model.mention = Some(Mention {
        paths: paths.iter().map(|path| (*path).to_owned()).collect(),
        waiting: false,
        selected: 0,
    });
    model
}

/// Every row's whole text.
fn painted(rows: &[Row]) -> Vec<String> {
    rows.iter().map(Row::text).collect()
}

/// The pane names itself and lists the paths under it, the first one marked.
#[test]
fn the_pane_lists_the_paths_it_offers() {
    let model = offering(&["src/main.rs", "src/lib.rs"]);
    let rows = rows(&model, 60, Charset::Utf8);
    let text = painted(&rows);
    assert!(text[0].contains(TITLE), "{text:?}");
    assert!(
        text.iter().any(|row| row.contains("src/main.rs")),
        "{text:?}"
    );
    assert!(
        text.iter().any(|row| row.contains("src/lib.rs")),
        "{text:?}"
    );
    assert!(
        text[1].contains('▸'),
        "the selected row carries the mark: {text:?}"
    );
    assert!(text.last().unwrap().contains("Tab insert"), "{text:?}");
}

/// No row is wider than the pane it is painted in, at either review size.
#[test]
fn no_row_is_wider_than_the_pane() {
    let model = offering(&["a/very/long/path/that/keeps/going/and/going/main/file.rs"]);
    for width in [60usize, 100] {
        for row in rows(&model, width, Charset::Utf8) {
            assert!(
                row.text().chars().count() <= width,
                "a {width}-column pane holds every row: {:?}",
                row.text()
            );
        }
    }
}

/// The list paints at most its row budget: a workspace of thousands is walked
/// with the arrows, not painted whole.
#[test]
fn the_pane_paints_at_most_its_row_budget() {
    let paths: Vec<String> = (0..MAX_VISIBLE + 5)
        .map(|index| format!("src/f{index}.rs"))
        .collect();
    let model = offering(&paths.iter().map(String::as_str).collect::<Vec<_>>());
    let rows = rows(&model, 80, Charset::Utf8);
    let painted_paths = rows
        .iter()
        .filter(|row| row.text().contains("src/f"))
        .count();
    assert_eq!(painted_paths, MAX_VISIBLE, "{painted_paths}");
}

/// The pane says what it is doing when it has nothing to show — while the walk
/// runs, and when the workspace offers nothing that matches.
#[test]
fn the_pane_says_what_it_is_doing() {
    let mut model = Model::default();
    model.push('@');
    model.mention = Some(Mention {
        waiting: true,
        ..Mention::default()
    });
    let text = painted(&rows(&model, 60, Charset::Utf8));
    assert!(text.iter().any(|row| row.contains(FINDING)), "{text:?}");
    let mut model = offering(&["src/main.rs"]);
    model.insert("zzz");
    let text = painted(&rows(&model, 60, Charset::Utf8));
    assert!(text.iter().any(|row| row.contains(NO_MATCH)), "{text:?}");
}

/// `--ascii` is a first-class shape: no glyph of the frame survives it.
#[test]
fn the_pane_draws_in_ascii() {
    let model = offering(&["src/main.rs"]);
    let rows = rows(&model, 60, Charset::Ascii);
    for row in &rows {
        let text = row.text();
        assert!(
            text.is_ascii(),
            "an ASCII terminal draws ASCII only: {text:?}"
        );
    }
    assert!(
        painted(&rows).last().unwrap().contains(HINT_ASCII),
        "the footer keeps its words: {:?}",
        painted(&rows)
    );
}
