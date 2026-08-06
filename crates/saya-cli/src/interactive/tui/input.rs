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
mod input_tests;
