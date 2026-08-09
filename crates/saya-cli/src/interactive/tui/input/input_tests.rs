use super::InputBuffer;

fn assert_cursor_invariant(buf: &InputBuffer) {
    assert!(
        buf.cursor() <= buf.text().chars().count(),
        "Cursor invariant violated: cursor = {}, char_count = {}",
        buf.cursor(),
        buf.text().chars().count()
    );
}

#[test]
fn test_insert_and_cursor_advance() {
    let mut buf = InputBuffer::new();
    assert!(buf.is_empty());
    assert_eq!(buf.cursor(), 0);

    buf.insert_char('a');
    assert_eq!(buf.text(), "a");
    assert_eq!(buf.cursor(), 1);

    buf.insert_str("bc");
    assert_eq!(buf.text(), "abc");
    assert_eq!(buf.cursor(), 3);

    buf.insert_newline();
    assert_eq!(buf.text(), "abc\n");
    assert_eq!(buf.cursor(), 4);

    assert_cursor_invariant(&buf);
}

#[test]
fn test_backspace_and_delete_at_edges() {
    let mut buf = InputBuffer::new();
    buf.backspace();
    assert_eq!(buf.text(), "");
    assert_eq!(buf.cursor(), 0);

    buf.delete();
    assert_eq!(buf.text(), "");
    assert_eq!(buf.cursor(), 0);

    buf.set_text("a");
    assert_eq!(buf.cursor(), 1);
    buf.delete();
    assert_eq!(buf.text(), "a");
    assert_eq!(buf.cursor(), 1);

    buf.backspace();
    assert_eq!(buf.text(), "");
    assert_eq!(buf.cursor(), 0);

    assert_cursor_invariant(&buf);
}

#[test]
fn test_left_right_clamping() {
    let mut buf = InputBuffer::new();
    buf.set_text("hi");
    assert_eq!(buf.cursor(), 2);

    buf.move_right();
    assert_eq!(buf.cursor(), 2);

    buf.move_left();
    assert_eq!(buf.cursor(), 1);

    buf.move_left();
    assert_eq!(buf.cursor(), 0);

    buf.move_left();
    assert_eq!(buf.cursor(), 0);

    assert_cursor_invariant(&buf);
}

#[test]
fn test_word_motions_across_multiple_spaces() {
    let mut buf = InputBuffer::new();
    buf.set_text("  hello   world  ");
    assert_eq!(buf.cursor(), 17);

    buf.move_word_left();
    assert_eq!(buf.cursor(), 10);

    buf.move_word_left();
    assert_eq!(buf.cursor(), 2);

    buf.move_word_left();
    assert_eq!(buf.cursor(), 0);

    buf.move_word_left();
    assert_eq!(buf.cursor(), 0);

    buf.move_word_right();
    assert_eq!(buf.cursor(), 7);

    buf.move_word_right();
    assert_eq!(buf.cursor(), 15);

    buf.move_word_right();
    assert_eq!(buf.cursor(), 17);

    buf.move_word_right();
    assert_eq!(buf.cursor(), 17);

    assert_cursor_invariant(&buf);
}

#[test]
fn test_home_and_end_on_middle_line() {
    let mut buf = InputBuffer::new();
    buf.set_text("line1\nline2\nline3");

    buf.move_home();
    assert_eq!(buf.cursor(), 12);

    // Put cursor in middle of "line2" (index 8: 'n')
    buf.cursor = 8;
    buf.move_home();
    assert_eq!(buf.cursor(), 6);

    buf.cursor = 8;
    buf.move_end();
    assert_eq!(buf.cursor(), 11);

    assert_cursor_invariant(&buf);
}

#[test]
fn test_multibyte_correctness() {
    let mut buf = InputBuffer::new();
    buf.set_text("café");
    assert_eq!(buf.cursor(), 4);

    buf.move_left();
    assert_eq!(buf.cursor(), 3);

    buf.insert_str("s");
    assert_eq!(buf.text(), "cafsé");
    assert_eq!(buf.cursor(), 4);

    buf.backspace();
    assert_eq!(buf.text(), "café");
    assert_eq!(buf.cursor(), 3);

    buf.delete();
    assert_eq!(buf.text(), "caf");
    assert_eq!(buf.cursor(), 3);

    buf.clear();
    buf.set_text("日本");
    assert_eq!(buf.cursor(), 2);

    buf.backspace();
    assert_eq!(buf.text(), "日");
    assert_eq!(buf.cursor(), 1);

    assert_cursor_invariant(&buf);
}

#[test]
fn test_lines_with_and_without_trailing_newline() {
    let mut buf = InputBuffer::new();
    buf.set_text("a\nb");
    assert_eq!(buf.lines(), vec!["a", "b"]);

    buf.set_text("a\nb\n");
    assert_eq!(buf.lines(), vec!["a", "b", ""]);

    buf.clear();
    assert_eq!(buf.lines(), vec![""]);

    assert_cursor_invariant(&buf);
}

#[test]
fn test_cursor_line_col_multiline() {
    let mut buf = InputBuffer::new();
    buf.set_text("abc\ndef\nghi");

    buf.cursor = 0;
    assert_eq!(buf.cursor_line_col(), (0, 0));

    buf.cursor = 2;
    assert_eq!(buf.cursor_line_col(), (0, 2));

    buf.cursor = 3;
    assert_eq!(buf.cursor_line_col(), (0, 3));

    buf.cursor = 4;
    assert_eq!(buf.cursor_line_col(), (1, 0));

    buf.cursor = 6;
    assert_eq!(buf.cursor_line_col(), (1, 2));

    buf.cursor = 7;
    assert_eq!(buf.cursor_line_col(), (1, 3));

    buf.cursor = 8;
    assert_eq!(buf.cursor_line_col(), (2, 0));

    buf.cursor = 11;
    assert_eq!(buf.cursor_line_col(), (2, 3));

    assert_cursor_invariant(&buf);
}

#[test]
fn test_kill_to_line_end_mid_line() {
    let mut buf = InputBuffer::new();
    buf.set_text("first line\nsecond line\nthird line");

    // Cursor at 18 (in "second line", before 'l' in "line")
    buf.cursor = 18;
    buf.kill_to_line_end();
    assert_eq!(buf.text(), "first line\nsecond \nthird line");
    assert_eq!(buf.cursor(), 18);

    assert_cursor_invariant(&buf);
}
