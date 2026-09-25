/// A multi-line text buffer with a cursor, addressed by a flat char index.
#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub(crate) struct InputBuffer {
    text: String,  // full text, may contain '\n'
    cursor: usize, // cursor position as a CHAR index in [0, char_count]
}

mod cursor;
mod edit;
mod view;
pub(crate) mod wrap;

#[cfg(test)]
mod input_tests;

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
}
