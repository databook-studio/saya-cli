/// The role/style class of a transcript block. The renderer maps these to colors.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BlockKind {
    User,
    Assistant,
    System,
    Error,
    Tool,
}

/// One logical entry in the transcript. `text` may contain '\n'.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub(crate) struct Block {
    pub(crate) kind: BlockKind,
    pub(crate) text: String,
}

/// Scrollback with soft-wrapping and a bottom-anchored scroll offset.
#[allow(dead_code)]
#[derive(Debug, Default)]
pub(crate) struct Transcript {
    blocks: Vec<Block>,
    /// Lines scrolled UP from the bottom. 0 = following the tail (newest visible).
    scroll_up: usize,
}

#[allow(dead_code)]
impl Transcript {
    /// Creates a new empty transcript.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Appends a block. If currently following the tail (`scroll_up == 0`), keeps following it.
    pub(crate) fn push(&mut self, kind: BlockKind, text: impl Into<String>) {
        self.blocks.push(Block {
            kind,
            text: text.into(),
        });
    }

    /// Appends `delta` to the last block if it has the same `kind`, or pushes a new block.
    pub(crate) fn append_delta(&mut self, kind: BlockKind, delta: &str) {
        if let Some(last) = self.blocks.last_mut().filter(|last| last.kind == kind) {
            last.text.push_str(delta);
            return;
        }
        self.push(kind, delta);
    }

    /// Returns `true` if the transcript contains no blocks.
    pub(crate) fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    /// Returns a slice of all blocks in the transcript.
    pub(crate) fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    /// Soft-wraps every block to `width` columns (by character count).
    pub(crate) fn wrapped(&self, width: usize) -> Vec<(BlockKind, String)> {
        let effective_width = width.max(1);
        let mut lines = Vec::new();
        for block in &self.blocks {
            for raw_line in block.text.split('\n') {
                if raw_line.is_empty() {
                    lines.push((block.kind, String::new()));
                } else {
                    let chars: Vec<char> = raw_line.chars().collect();
                    for chunk in chars.chunks(effective_width) {
                        lines.push((block.kind, chunk.iter().collect()));
                    }
                }
            }
        }
        lines
    }

    /// Returns the total number of wrapped lines at the given `width`.
    pub(crate) fn total_lines(&self, width: usize) -> usize {
        self.wrapped(width).len()
    }

    /// Returns the visible slice of wrapped lines for a given viewport width and height.
    pub(crate) fn view(&self, width: usize, height: usize) -> Vec<(BlockKind, String)> {
        if height == 0 {
            return Vec::new();
        }
        let lines = self.wrapped(width);
        let total = lines.len();
        if total <= height {
            return lines;
        }
        let max_scroll = total - height;
        let effective_scroll = self.scroll_up.min(max_scroll);
        let start = total - height - effective_scroll;
        lines[start..start + height].to_vec()
    }

    /// Increases `scroll_up` by `n`, clamped to max scroll offset.
    pub(crate) fn scroll_up(&mut self, n: usize, width: usize, height: usize) {
        let total = self.total_lines(width);
        let max_scroll = total.saturating_sub(height);
        self.scroll_up = self.scroll_up.saturating_add(n).min(max_scroll);
    }

    /// Decreases `scroll_up` by `n`, saturating at 0.
    pub(crate) fn scroll_down(&mut self, n: usize) {
        self.scroll_up = self.scroll_up.saturating_sub(n);
    }

    /// Resets `scroll_up` to 0 (following the tail).
    pub(crate) fn scroll_to_bottom(&mut self) {
        self.scroll_up = 0;
    }

    /// Whether the view is pinned to the newest content.
    pub(crate) fn is_following_tail(&self) -> bool {
        self.scroll_up == 0
    }

    /// Returns `(total_lines, index_of_first_visible_line)` for a scrollbar.
    pub(crate) fn scroll_metrics(&self, width: usize, height: usize) -> (usize, usize) {
        let total = self.total_lines(width);
        if total <= height {
            return (total, 0);
        }
        let max_scroll = total - height;
        let effective = self.scroll_up.min(max_scroll);
        (total, total - height - effective)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_push_and_append_delta() {
        let mut t = Transcript::new();
        assert!(t.is_empty() && t.blocks().is_empty());
        t.push(BlockKind::User, "Hello");
        t.push(BlockKind::Assistant, "Hi there");
        assert_eq!(t.blocks().len(), 2);

        t.append_delta(BlockKind::Assistant, "!");
        assert_eq!(t.blocks()[1].text, "Hi there!");
        t.append_delta(BlockKind::User, "Bye");
        assert_eq!(t.blocks().len(), 3);
    }

    #[test]
    fn test_wrapping() {
        let mut t = Transcript::new();
        let long_str = "1234567890abcdefghij";
        t.push(BlockKind::System, long_str);
        let lines = t.wrapped(10);
        let concat: String = lines.iter().map(|(_, s)| s.as_str()).collect();
        assert_eq!(concat, long_str);

        let mut t2 = Transcript::new();
        t2.push(BlockKind::Error, "a\n\nb\n");
        assert_eq!(t2.wrapped(10).len(), 4);

        let mut t3 = Transcript::new();
        t3.push(BlockKind::Tool, "日日日日日");
        assert_eq!(t3.wrapped(2).len(), 3);

        let mut t4 = Transcript::new();
        t4.push(BlockKind::User, "abc");
        assert_eq!(t4.wrapped(0).len(), 3);
    }

    #[test]
    fn test_view_and_scrolling() {
        let mut t = Transcript::new();
        t.push(BlockKind::User, "l1\nl2\nl3\nl4\nl5");

        let texts = |v: Vec<(BlockKind, String)>| -> Vec<String> {
            v.into_iter().map(|(_, s)| s).collect()
        };

        assert_eq!(texts(t.view(10, 3)), vec!["l3", "l4", "l5"]);

        t.scroll_up(1, 10, 3);
        assert_eq!(texts(t.view(10, 3)), vec!["l2", "l3", "l4"]);

        t.scroll_up(100, 10, 3);
        assert_eq!(texts(t.view(10, 3)), vec!["l1", "l2", "l3"]);

        t.scroll_down(1);
        assert_eq!(t.view(10, 3)[0].1, "l2");

        t.scroll_down(10);
        assert_eq!(t.view(10, 3)[0].1, "l3");

        t.scroll_up(2, 10, 3);
        t.scroll_to_bottom();
        assert_eq!(t.view(10, 3)[0].1, "l3");
    }

    #[test]
    fn test_edge_cases() {
        let mut empty = Transcript::new();
        assert!(empty.view(10, 5).is_empty() && empty.total_lines(10) == 0);
        empty.scroll_up(5, 10, 5);
        empty.scroll_down(2);
        empty.scroll_to_bottom();

        let mut t = Transcript::new();
        t.push(BlockKind::User, "hello");
        assert!(t.view(10, 0).is_empty());
    }
}
