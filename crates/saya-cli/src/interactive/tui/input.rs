/// A multi-line text buffer with a cursor, addressed by a flat char index.
#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub(crate) struct InputBuffer {
    text: String,  // full text, may contain '\n'
    cursor: usize, // cursor position as a CHAR index in [0, char_count]
}

/// Helper function to convert a char index to a byte offset safely without panicking.
#[allow(dead_code)]
fn char_to_byte_idx(text: &str, char_idx: usize) -> usize {
    match text.char_indices().nth(char_idx) {
        Some((idx, _)) => idx,
        None => text.len(),
    }
}

#[allow(dead_code)]
impl InputBuffer {
    /// Creates a new empty input buffer.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Returns the full text content of the buffer.
    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    /// Returns true if the buffer contains no text.
    pub(crate) fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Returns the cursor position as a char index.
    pub(crate) fn cursor(&self) -> usize {
        self.cursor
    }

    /// Clears all text and resets the cursor position to 0.
    pub(crate) fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
    }

    /// Replaces the buffer text and puts the cursor at the end.
    pub(crate) fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
        self.cursor = self.text.chars().count();
    }

    /// Inserts a single character at the cursor position and advances the cursor by 1.
    pub(crate) fn insert_char(&mut self, c: char) {
        let byte_idx = char_to_byte_idx(&self.text, self.cursor);
        self.text.insert(byte_idx, c);
        self.cursor += 1;
    }

    /// Inserts a string slice at the cursor position and advances the cursor by its char count.
    pub(crate) fn insert_str(&mut self, s: &str) {
        let byte_idx = char_to_byte_idx(&self.text, self.cursor);
        self.text.insert_str(byte_idx, s);
        self.cursor += s.chars().count();
    }

    /// Inserts a newline character at the cursor position.
    pub(crate) fn insert_newline(&mut self) {
        self.insert_char('\n');
    }

    /// Deletes the character before the cursor (no-op at start).
    pub(crate) fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let start_byte = char_to_byte_idx(&self.text, self.cursor - 1);
        let end_byte = char_to_byte_idx(&self.text, self.cursor);
        self.text.replace_range(start_byte..end_byte, "");
        self.cursor -= 1;
    }

    /// Deletes the character at the cursor position (no-op at end).
    pub(crate) fn delete(&mut self) {
        let char_count = self.text.chars().count();
        if self.cursor >= char_count {
            return;
        }
        let start_byte = char_to_byte_idx(&self.text, self.cursor);
        let end_byte = char_to_byte_idx(&self.text, self.cursor + 1);
        self.text.replace_range(start_byte..end_byte, "");
    }

    /// Moves the cursor left by 1 character, clamped at 0.
    pub(crate) fn move_left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    /// Moves the cursor right by 1 character, clamped at the end of the text.
    pub(crate) fn move_right(&mut self) {
        let char_count = self.text.chars().count();
        if self.cursor < char_count {
            self.cursor += 1;
        }
    }

    /// Moves the cursor left by one word (skipping whitespace, then non-whitespace).
    pub(crate) fn move_word_left(&mut self) {
        let chars: Vec<char> = self.text.chars().collect();
        let mut i = self.cursor;
        while i > 0 && chars[i - 1].is_whitespace() {
            i -= 1;
        }
        while i > 0 && !chars[i - 1].is_whitespace() {
            i -= 1;
        }
        self.cursor = i;
    }

    /// Moves the cursor right by one word (skipping whitespace, then non-whitespace).
    pub(crate) fn move_word_right(&mut self) {
        let chars: Vec<char> = self.text.chars().collect();
        let len = chars.len();
        let mut i = self.cursor;
        while i < len && chars[i].is_whitespace() {
            i += 1;
        }
        while i < len && !chars[i].is_whitespace() {
            i += 1;
        }
        self.cursor = i;
    }

    /// Moves the cursor to the start of the current line.
    pub(crate) fn move_home(&mut self) {
        let chars: Vec<char> = self.text.chars().collect();
        let mut i = self.cursor;
        while i > 0 {
            if chars[i - 1] == '\n' {
                break;
            }
            i -= 1;
        }
        self.cursor = i;
    }

    /// Moves the cursor to the end of the current line.
    pub(crate) fn move_end(&mut self) {
        let chars: Vec<char> = self.text.chars().collect();
        let len = chars.len();
        let mut i = self.cursor;
        while i < len {
            if chars[i] == '\n' {
                break;
            }
            i += 1;
        }
        self.cursor = i;
    }

    /// Deletes from the cursor to the end of the current line.
    pub(crate) fn kill_to_line_end(&mut self) {
        let char_count = self.text.chars().count();
        if self.cursor >= char_count {
            return;
        }
        let chars: Vec<char> = self.text.chars().collect();
        let mut end = self.cursor;
        while end < char_count && chars[end] != '\n' {
            end += 1;
        }
        if end == self.cursor && end < char_count {
            end += 1;
        }
        if end > self.cursor {
            let start_byte = char_to_byte_idx(&self.text, self.cursor);
            let end_byte = char_to_byte_idx(&self.text, end);
            self.text.replace_range(start_byte..end_byte, "");
        }
    }

    /// Deletes the word before the cursor (Ctrl+W).
    pub(crate) fn delete_word_left(&mut self) {
        let end = self.cursor;
        self.move_word_left();
        let start = self.cursor;
        if start < end {
            let start_byte = char_to_byte_idx(&self.text, start);
            let end_byte = char_to_byte_idx(&self.text, end);
            self.text.replace_range(start_byte..end_byte, "");
        }
    }

    /// Deletes from the start of the current line to the cursor (Ctrl+U).
    pub(crate) fn kill_to_line_start(&mut self) {
        let end = self.cursor;
        self.move_home();
        let start = self.cursor;
        if start < end {
            let start_byte = char_to_byte_idx(&self.text, start);
            let end_byte = char_to_byte_idx(&self.text, end);
            self.text.replace_range(start_byte..end_byte, "");
        }
    }

    /// Returns the text split on '\n'.
    pub(crate) fn lines(&self) -> Vec<&str> {
        self.text.split('\n').collect()
    }

    /// Returns the (line index, column-in-chars) position of the cursor for rendering.
    pub(crate) fn cursor_line_col(&self) -> (usize, usize) {
        let mut line = 0;
        let mut col = 0;
        for c in self.text.chars().take(self.cursor) {
            if c == '\n' {
                line += 1;
                col = 0;
            } else {
                col += 1;
            }
        }
        (line, col)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
