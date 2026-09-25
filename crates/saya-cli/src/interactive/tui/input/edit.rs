use super::{InputBuffer, char_to_byte_idx};

#[allow(dead_code)]
impl InputBuffer {
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
}
