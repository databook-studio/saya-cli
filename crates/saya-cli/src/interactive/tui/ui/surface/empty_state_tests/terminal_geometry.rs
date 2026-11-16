use super::*;

/// The mascot's body glyph marks the figure: no other home-screen text uses
/// it.
const MASCOT_GLYPH: char = '\u{2588}';

/// The character column where `needle` starts in `line`.
fn col_of(line: &str, needle: &str) -> usize {
    let chars: Vec<char> = line.chars().collect();
    let needle: Vec<char> = needle.chars().collect();
    chars
        .windows(needle.len())
        .position(|window| window == needle.as_slice())
        .unwrap_or_else(|| panic!("{needle:?} is not in the line: {line:?}"))
}

/// The character row where `needle` first paints.
fn row_of(buffer: &str, needle: &str) -> usize {
    buffer
        .lines()
        .position(|line| line.contains(needle))
        .unwrap_or_else(|| panic!("{needle:?} paints:\n{buffer}"))
}

/// A wide terminal splits the box into two columns with a vertical rule.
#[test]
fn a_wide_terminal_splits_into_two_columns() {
    let buffer = splash(120, 36, &[], true);
    let rule_count = buffer
        .lines()
        .map(|line| line.matches('\u{2502}').count())
        .max()
        .unwrap_or(0);
    assert_eq!(
        rule_count, 3,
        "two box sides plus the vertical rule separate the columns:\n{buffer}"
    );
    assert!(
        col_of(
            buffer.lines().nth(row_of(&buffer, WELCOME)).unwrap(),
            WELCOME
        ) < col_of(
            buffer
                .lines()
                .nth(row_of(&buffer, GETTING_STARTED))
                .unwrap(),
            GETTING_STARTED
        ),
        "the welcome column is left of the guidance column:\n{buffer}"
    );
    assert!(
        buffer.contains(MASCOT_GLYPH),
        "the mascot paints:\n{buffer}"
    );
}

/// The box is content-sized and anchored at the top of the transcript
/// area: it no longer fills the terminal height with empty rows.
#[test]
fn the_box_is_compact_and_anchored_at_the_top() {
    let buffer = splash(120, 34, &[], true);
    assert_eq!(
        row_of(&buffer, "\u{256d}"),
        0,
        "the top border anchors at the top of the pane:\n{buffer}"
    );
    let bottom = row_of(&buffer, "\u{2570}");
    assert!(
        (10..20).contains(&bottom),
        "the box is content-sized, not full height (bottom row {bottom}):\n{buffer}"
    );
}

/// Both columns are top-aligned: the first line of each starts on the same
/// row.
#[test]
fn both_columns_start_on_the_same_row() {
    let buffer = splash(120, 34, &[], true);
    assert_eq!(
        row_of(&buffer, WELCOME),
        row_of(&buffer, GETTING_STARTED),
        "the two columns are top-aligned:\n{buffer}"
    );
}

/// Every column keeps two cells of horizontal padding: text touches neither
/// the box borders nor the vertical rule.
#[test]
fn the_content_is_padded_off_the_borders_and_divider() {
    let buffer = splash(120, 34, &[], true);
    let guidance_line = buffer
        .lines()
        .nth(row_of(&buffer, GETTING_STARTED))
        .unwrap();
    let rules: Vec<usize> = guidance_line
        .chars()
        .enumerate()
        .filter(|(_, ch)| *ch == '\u{2502}')
        .map(|(column, _)| column)
        .collect();
    assert_eq!(rules.len(), 3, "both borders and the divider:\n{buffer}");
    let heading = col_of(guidance_line, GETTING_STARTED);
    assert!(
        heading >= rules[1] + 3,
        "two cells of padding after the divider:\n{buffer}"
    );
    assert!(
        heading + GETTING_STARTED.chars().count() <= rules[2] - 2,
        "two cells of padding before the right border:\n{buffer}"
    );

    let welcome_line = buffer.lines().nth(row_of(&buffer, WELCOME)).unwrap();
    assert!(
        col_of(welcome_line, WELCOME) >= 3,
        "two cells of padding after the left border:\n{buffer}"
    );
}

/// One blank row sits between the title border and the first content row.
#[test]
fn one_blank_row_sits_under_the_title_border() {
    let buffer = splash(120, 34, &[], true);
    let title = buffer.lines().next().expect("a title row");
    assert!(
        title.contains("saya v"),
        "the title names the version:\n{buffer}"
    );
    let under = buffer.lines().nth(1).expect("a row under the title");
    assert!(
        !under.chars().any(|ch| ch.is_alphanumeric()),
        "the row under the title is blank:\n{buffer}"
    );
}

/// A narrow terminal stacks the two columns vertically inside the box.
#[test]
fn a_narrow_terminal_stacks_the_columns() {
    let buffer = splash(60, 20, &[], true);
    let rule_count = buffer
        .lines()
        .map(|line| line.matches('\u{2502}').count())
        .max()
        .unwrap_or(0);
    assert_eq!(
        rule_count, 2,
        "only the box sides, no vertical rule, in the stacked layout:\n{buffer}"
    );
    assert!(
        buffer.contains('\u{2500}'),
        "a horizontal rule separates the stacked sections:\n{buffer}"
    );
    assert!(
        row_of(&buffer, WELCOME) < row_of(&buffer, GETTING_STARTED),
        "the welcome section stacks above the guidance:\n{buffer}"
    );
    assert!(
        buffer.contains("AI        ready"),
        "the Status block survives the stack:\n{buffer}"
    );
}

/// A short terminal drops the mascot before anything else, and never drops
/// the Status block.
#[test]
fn a_short_terminal_drops_the_mascot_first() {
    let buffer = splash(120, 12, &[], true);
    assert!(
        !buffer.contains(MASCOT_GLYPH),
        "the mascot is the first thing a short terminal drops:\n{buffer}"
    );
    assert!(
        buffer.contains("AI        ready") && buffer.contains("Project   ready"),
        "the Status block is never dropped:\n{buffer}"
    );
}

/// The old footer line is gone: the status bar already carries `? for help`.
#[test]
fn the_extra_footer_line_is_gone() {
    for (width, height) in [(120, 34), (80, 24), (60, 20), (120, 12)] {
        let buffer = splash(width, height, &[], true);
        assert!(
            !buffer.contains("? for shortcuts"),
            "no extra footer at {width}x{height}:\n{buffer}"
        );
    }
}

/// The whole composed frame through the real `ui::draw` at the smoke size:
/// the box, the welcome, and the status all paint together.
#[test]
fn the_composed_first_screen_paints_the_home_screen() {
    let mut app = empty_app();
    app.profiles.clear();
    let mut status = fixed_status();
    status.workspace_root = None;
    let buffer = render_buffer(&app, &status, 80, 24);
    assert!(buffer.contains(WELCOME), "the welcome paints:\n{buffer}");
    assert!(
        buffer.contains("none yet — optional"),
        "the optional-absence database line paints:\n{buffer}"
    );
    insta::assert_snapshot!(buffer);
}
