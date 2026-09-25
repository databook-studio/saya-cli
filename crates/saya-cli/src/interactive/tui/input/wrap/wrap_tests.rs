use super::super::cursor::cursor_visual;
use super::{visual_row_count, wrap_line};

/// A line exactly the inner width wraps to a single row — no spurious extra
/// row. (Spec test list item 1.)
#[test]
fn line_exactly_inner_width_is_one_row() {
    assert_eq!(wrap_line("abcdefghij", 10), vec!["abcdefghij"]);
    assert_eq!(visual_row_count("abcdefghij", 10), 1);
}

/// A line one character over wraps to exactly two visual rows, and the cursor
/// at the end lands on the second row. (Spec test list items 1 & 2.)
#[test]
fn one_char_over_is_two_rows_cursor_on_second() {
    assert_eq!(wrap_line("abcdefghijk", 10), vec!["abcdefghij", "k"]);
    assert_eq!(visual_row_count("abcdefghijk", 10), 2);
    // cursor at the very end (char index 11) -> row 1, col 1 (after "k")
    assert_eq!(cursor_visual("abcdefghijk", 10, 11), (1, 1));
}

/// Cursor at the boundary between the two visual rows lands at the start of the
/// second row, not past the end of the first.
#[test]
fn cursor_at_wrap_boundary_starts_second_row() {
    assert_eq!(cursor_visual("abcdefghijk", 10, 10), (1, 0));
}

/// Cursor at the very end of a wrapped line: the last char's position + 1.
/// (Spec test list item 3.)
#[test]
fn cursor_at_end_of_wrapped_line() {
    // "abc def ghi" at width 7 -> ["abc def", "ghi"]; cursor at end (11) -> (1, 3)
    assert_eq!(wrap_line("abc def ghi", 7), vec!["abc def", "ghi"]);
    assert_eq!(cursor_visual("abc def ghi", 7, 11), (1, 3));
}

/// Cursor at position 0 of an empty buffer maps to (0, 0). The render path's
/// placeholder branch handles empty input separately; this guarantees the
/// mapping itself degrades to the origin. (Spec test list item 4.)
#[test]
fn cursor_at_start_of_empty_buffer_is_origin() {
    assert_eq!(cursor_visual("", 10, 0), (0, 0));
    assert_eq!(visual_row_count("", 10), 1);
    assert_eq!(wrap_line("", 10), vec![""]);
}

/// Prose wraps on word boundaries, keeping as many words per line as fit.
#[test]
fn prose_wraps_on_word_boundaries() {
    assert_eq!(
        wrap_line("the quick brown fox", 10),
        vec!["the quick", "brown fox"]
    );
    // cursor after "the quick " (index 10, the space) is the break point and
    // lands at the end of row 0.
    assert_eq!(cursor_visual("the quick brown fox", 10, 9), (0, 9));
}

/// An over-long unbreakable token hard-breaks at the width.
#[test]
fn over_long_word_hard_breaks() {
    assert_eq!(
        wrap_line("abcdefghijklmno", 10),
        vec!["abcdefghij", "klmno"]
    );
    assert_eq!(cursor_visual("abcdefghijklmno", 10, 15), (1, 5));
}

/// More visual rows than MAX_INPUT_ROWS: the wrap produces them all; capping
/// and scroll-to-cursor happen in the render path, not here. This pins the
/// count the render path consumes. (Spec test list item 6.)
#[test]
fn many_visual_rows_when_line_exceeds_max() {
    let long = "a".repeat(80);
    assert_eq!(visual_row_count(&long, 10), 8);
    // cursor at end -> last row, col 0 (80 / 10 = 8 rows, last row full => col 0)
    assert_eq!(cursor_visual(&long, 10, 80), (7, 10));
}

/// Multi-byte characters: the buffer indexes by char, the wrap counts by char.
/// A 2-char CJK string at width 2 wraps to one row **by char count** — NOT by
/// display width (each CJK char occupies 2 cells). This pins the known
/// limitation: display width is not handled. (Spec test list item 5.)
#[test]
fn multibyte_wraps_by_char_count_not_display_width() {
    assert_eq!(wrap_line("日本", 2), vec!["日本"]);
    assert_eq!(visual_row_count("日本", 2), 1);
    // A CJK string that *would* fit by char count at width 4 but occupies 4
    // cells: still one row by char count (the limitation).
    assert_eq!(wrap_line("日本語字", 4), vec!["日本語字"]);
}

/// Multi-line input: each logical line wraps independently and the visual row
/// is the sum of the preceding lines' wrapped rows plus the row within the
/// cursor's logical line.
#[test]
fn logical_newlines_compose_with_wrap() {
    // "abcd\nefghijk" at width 5: line0 "abcd" (1 row), line1 "efghijk" ->
    // ["efghi", "jk"] (2 rows). The text is 12 chars (indices 0..11, end=12):
    //   0=a 1=b 2=c 3=d 4=\n 5=e 6=f 7=g 8=h 9=i 10=j 11=k
    // cursor=11 is on 'k' (last char) -> visual row 2, col 1.
    // cursor=12 (past the end) -> visual row 2, col 2 (after "jk").
    let text = "abcd\nefghijk";
    assert_eq!(visual_row_count(text, 5), 3);
    assert_eq!(cursor_visual(text, 5, 11), (2, 1));
    assert_eq!(cursor_visual(text, 5, 12), (2, 2));
    // Cursor at the newline itself (index 4) is at the end of logical line 0
    // (before the newline is consumed), matching `cursor_line_col` -> (0, 4).
    assert_eq!(cursor_visual(text, 5, 4), (0, 4));
    // Cursor just past the newline (index 5) -> start of logical line 1 ->
    // visual row 1, col 0.
    assert_eq!(cursor_visual(text, 5, 5), (1, 0));
}

/// Leading whitespace is preserved on a wrapped line (trim:false semantics).
#[test]
fn leading_whitespace_is_preserved() {
    assert_eq!(wrap_line("    abc def", 7), vec!["    abc", "def"]);
}
