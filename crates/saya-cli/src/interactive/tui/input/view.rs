use super::{InputBuffer, cursor, wrap};

#[allow(dead_code)]
impl InputBuffer {
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

    /// Visual lines of the whole buffer wrapped to `width` (one logical line
    /// may produce several). See [`wrap::wrap_line`].
    pub(crate) fn wrapped_lines(&self, width: usize) -> Vec<String> {
        self.lines()
            .into_iter()
            .flat_map(|line| wrap::wrap_line(line, width))
            .collect()
    }

    /// Visual `(row, col)` of the cursor against a wrap of `width` columns.
    /// See [`cursor::cursor_visual`].
    pub(crate) fn cursor_visual(&self, width: usize) -> (usize, usize) {
        cursor::cursor_visual(&self.text, width, self.cursor)
    }

    /// Number of visual rows the buffer occupies when wrapped to `width`.
    /// See [`wrap::visual_row_count`].
    pub(crate) fn visual_row_count(&self, width: usize) -> usize {
        wrap::visual_row_count(&self.text, width)
    }
}
