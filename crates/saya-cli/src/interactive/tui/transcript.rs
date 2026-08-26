use std::{cell::RefCell, rc::Rc};

const MAX_BLOCKS: usize = 5000;
const MAX_TOTAL_TEXT_BYTES: usize = 4 << 20;

type WrappedLines = Vec<(BlockKind, String)>;
type WrapCache = RefCell<Option<(usize, Rc<WrappedLines>)>>;

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BlockKind {
    User,
    Assistant,
    System,
    Error,
    Tool,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub(crate) struct Block {
    pub(crate) kind: BlockKind,
    pub(crate) text: String,
}

#[allow(dead_code)]
#[derive(Debug, Default)]
pub(crate) struct Transcript {
    blocks: Vec<Block>,
    scroll_up: usize,
    cache: WrapCache,
}

#[allow(dead_code)]
impl Transcript {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    fn invalidate_cache(&self) {
        *self.cache.borrow_mut() = None;
    }

    fn enforce_bounds(&mut self) {
        let mut bytes: usize = self.blocks.iter().map(|b| b.text.len()).sum();
        let mut drop = 0;
        while self.blocks.len().saturating_sub(drop) > MAX_BLOCKS
            || (bytes > MAX_TOTAL_TEXT_BYTES && drop < self.blocks.len())
        {
            bytes = bytes.saturating_sub(self.blocks[drop].text.len());
            drop += 1;
        }
        if drop > 0 {
            self.blocks.drain(..drop);
        }
    }

    pub(crate) fn push(&mut self, kind: BlockKind, text: impl Into<String>) {
        let text = text.into();
        self.blocks.push(Block { kind, text });
        self.enforce_bounds();
        self.invalidate_cache();
    }

    pub(crate) fn append_delta(&mut self, kind: BlockKind, delta: &str) {
        if let Some(last) = self.blocks.last_mut().filter(|last| last.kind == kind) {
            last.text.push_str(delta);
            self.enforce_bounds();
            self.invalidate_cache();
        } else {
            self.push(kind, delta);
        }
    }

    pub(crate) fn reformat_last(&mut self, kind: BlockKind, f: impl FnOnce(&str) -> String) {
        if let Some(block) = self.blocks.iter_mut().rev().find(|b| b.kind == kind) {
            block.text = f(&block.text);
            self.invalidate_cache();
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    pub(crate) fn clear(&mut self) {
        self.blocks.clear();
        self.scroll_up = 0;
        self.invalidate_cache();
    }

    pub(crate) fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    fn lines(&self, width: usize) -> Rc<WrappedLines> {
        let eff = width.max(1);
        if let Some((_, lines)) = self.cache.borrow().as_ref().filter(|(w, _)| *w == eff) {
            return Rc::clone(lines);
        }
        let mut lines = Vec::new();
        for block in &self.blocks {
            for raw in block.text.split('\n') {
                if raw.is_empty() {
                    lines.push((block.kind, String::new()));
                } else {
                    wrap_word_aware(raw, eff, block.kind, &mut lines);
                }
            }
        }
        let rc = Rc::new(lines);
        *self.cache.borrow_mut() = Some((eff, Rc::clone(&rc)));
        rc
    }

    pub(crate) fn wrapped(&self, width: usize) -> WrappedLines {
        (*self.lines(width)).clone()
    }

    pub(crate) fn total_lines(&self, width: usize) -> usize {
        self.lines(width).len()
    }

    pub(crate) fn view(&self, width: usize, height: usize) -> WrappedLines {
        if height == 0 {
            return Vec::new();
        }
        let lines = self.lines(width);
        let rem = lines.len().saturating_sub(height);
        if rem == 0 {
            return (*lines).clone();
        }
        let start = rem - self.scroll_up.min(rem);
        lines[start..start + height].to_vec()
    }

    pub(crate) fn scroll_up(&mut self, n: usize, width: usize, height: usize) {
        let max = self.total_lines(width).saturating_sub(height);
        self.scroll_up = self.scroll_up.saturating_add(n).min(max);
    }

    pub(crate) fn scroll_down(&mut self, n: usize) {
        self.scroll_up = self.scroll_up.saturating_sub(n);
    }

    pub(crate) fn scroll_to_bottom(&mut self) {
        self.scroll_up = 0;
    }

    pub(crate) fn is_following_tail(&self) -> bool {
        self.scroll_up == 0
    }

    pub(crate) fn scroll_metrics(&self, width: usize, height: usize) -> (usize, usize) {
        let total = self.total_lines(width);
        let rem = total.saturating_sub(height);
        if rem == 0 {
            return (total, 0);
        }
        (total, rem - self.scroll_up.min(rem))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transcript_all() {
        let mut t = Transcript::new();
        assert!(t.is_empty() && t.blocks().is_empty());
        t.push(BlockKind::User, "Hello");
        t.push(BlockKind::Assistant, "Hi there");
        t.append_delta(BlockKind::Assistant, "!");
        t.append_delta(BlockKind::User, "Bye");
        assert_eq!(t.blocks().len(), 3);
        assert_eq!(t.blocks()[1].text, "Hi there!");

        t.push(BlockKind::Assistant, "streaming answer");
        t.reformat_last(BlockKind::Assistant, |s| format!("[formatted: {s}]"));
        assert_eq!(t.blocks()[3].text, "[formatted: streaming answer]");

        let mut t2 = Transcript::new();
        t2.push(BlockKind::System, "1234567890abcdefghij");
        let s: String = t2.wrapped(10).iter().map(|(_, s)| s.as_str()).collect();
        assert_eq!(s, "1234567890abcdefghij");

        let (mut t3, mut t4) = (Transcript::new(), Transcript::new());
        t3.push(BlockKind::Error, "a\n\nb\n");
        t4.push(BlockKind::Tool, "日日日日日");
        assert!(t3.wrapped(10).len() == 4 && t4.wrapped(2).len() == 3);

        t.clear();
        t.push(BlockKind::User, "l1\nl2\nl3\nl4\nl5");
        let texts = |v: WrappedLines| v.into_iter().map(|(_, s)| s).collect::<Vec<_>>();
        assert_eq!(texts(t.view(10, 3)), ["l3", "l4", "l5"]);
        t.scroll_up(1, 10, 3);
        assert_eq!(texts(t.view(10, 3)), ["l2", "l3", "l4"]);
        t.scroll_up(100, 10, 3);
        assert_eq!(texts(t.view(10, 3)), ["l1", "l2", "l3"]);
        t.scroll_down(1);
        assert_eq!(t.view(10, 3)[0].1, "l2");
        t.scroll_down(10);
        assert_eq!(t.view(10, 3)[0].1, "l3");
        t.scroll_up(2, 10, 3);
        t.scroll_to_bottom();
        assert_eq!(t.view(10, 3)[0].1, "l3");

        let (mut empty, mut t_edge) = (Transcript::new(), Transcript::new());
        empty.scroll_up(5, 10, 5);
        empty.scroll_down(2);
        empty.scroll_to_bottom();
        t_edge.push(BlockKind::User, "hello");
        assert!(empty.view(10, 5).is_empty());
        assert_eq!(empty.total_lines(10), 0);
        assert!(t_edge.view(10, 0).is_empty());

        let mut tc = Transcript::new();
        tc.push(BlockKind::User, "hello world");
        let (l1, l2) = (tc.lines(10), tc.lines(10));
        assert!(Rc::ptr_eq(&l1, &l2) && tc.wrapped(10).len() == 2);

        tc.push(BlockKind::Assistant, "hi");
        let (l3, l4) = (tc.lines(10), tc.lines(20));
        assert!(!Rc::ptr_eq(&l2, &l3) && !Rc::ptr_eq(&l3, &l4));
        tc.append_delta(BlockKind::Assistant, " there");
        assert!(!Rc::ptr_eq(&l4, &tc.lines(20)));
        tc.clear();
        assert!(tc.lines(20).is_empty());

        let mut tb = Transcript::new();
        (0..MAX_BLOCKS + 100).for_each(|i| tb.push(BlockKind::User, format!("msg {i}")));
        assert_eq!(tb.blocks().len(), MAX_BLOCKS);
        assert_eq!(tb.blocks()[0].text, "msg 100");
        let last_text = format!("msg {}", MAX_BLOCKS + 99);
        assert_eq!(tb.blocks()[MAX_BLOCKS - 1].text, last_text);

        let mut tb2 = Transcript::new();
        tb2.push(BlockKind::User, "old block");
        let huge = "x".repeat(MAX_TOTAL_TEXT_BYTES);
        tb2.push(BlockKind::Assistant, huge.clone());
        assert_eq!(tb2.blocks().len(), 1);
        assert_eq!(tb2.blocks()[0].text, huge);
        assert_eq!(tb2.view(10, 1)[0].0, BlockKind::Assistant);
    }

    #[test]
    fn test_transcript_max_blocks_large_input_bound() {
        let mut t = Transcript::new();
        let total_pushed = MAX_BLOCKS + 5000;
        for i in 0..total_pushed {
            t.push(BlockKind::User, format!("block {i}"));
        }

        assert!(
            t.blocks().len() <= MAX_BLOCKS,
            "blocks length must be clamped to <= MAX_BLOCKS"
        );
        assert_eq!(t.blocks().len(), MAX_BLOCKS);

        let expected_first = format!("block {}", total_pushed - MAX_BLOCKS);
        let expected_last = format!("block {}", total_pushed - 1);

        assert_eq!(t.blocks().first().unwrap().text, expected_first);
        assert_eq!(t.blocks().last().unwrap().text, expected_last);

        let view_lines = t.view(80, 20);
        assert_eq!(
            view_lines.len(),
            20,
            "view(width, height) must return exactly height lines after stress"
        );

        let wrapped = t.wrapped(80);
        assert_eq!(
            wrapped.len(),
            t.total_lines(80),
            "wrapped length must match total_lines length"
        );
        assert_eq!(wrapped.len(), MAX_BLOCKS);
    }

    #[test]
    fn test_transcript_total_bytes_eviction() {
        let mut t = Transcript::new();
        t.push(BlockKind::User, "initial early message");

        let chunk_size = 1_500_000;
        let large_1 = "a".repeat(chunk_size);
        let large_2 = "b".repeat(chunk_size);
        let large_3 = "c".repeat(chunk_size);

        t.push(BlockKind::Assistant, large_1);
        t.push(BlockKind::Assistant, large_2);
        t.push(BlockKind::Assistant, large_3.clone());

        let total_bytes: usize = t.blocks().iter().map(|b| b.text.len()).sum();
        assert!(
            total_bytes <= MAX_TOTAL_TEXT_BYTES,
            "Total text bytes ({total_bytes}) must be <= MAX_TOTAL_TEXT_BYTES ({MAX_TOTAL_TEXT_BYTES})"
        );

        assert_ne!(
            t.blocks().first().unwrap().text,
            "initial early message",
            "Initial early message must be evicted"
        );
        assert_eq!(
            t.blocks().last().unwrap().text,
            large_3,
            "Newest pushed block must be retained"
        );
    }
}

/// Wraps one logical line to `width` chars, preferring the last space inside
/// the window so words are not split mid-word; over-long single tokens still
/// split (they have nowhere else to go).
fn wrap_word_aware(raw: &str, width: usize, kind: BlockKind, out: &mut Vec<(BlockKind, String)>) {
    let chars: Vec<char> = raw.chars().collect();
    let mut start = 0;
    while start < chars.len() {
        let remaining = chars.len() - start;
        if remaining <= width {
            out.push((kind, chars[start..].iter().collect()));
            break;
        }
        let window = &chars[start..start + width];
        // Last space in the window (never at position 0, or we would loop).
        let space = window
            .iter()
            .rposition(|c: &char| c.is_whitespace())
            .filter(|&index| index > 0);
        let (emit_end, next_start) = match space {
            // Break on the space: it ends this line (trimmed) and is skipped.
            Some(index) => (index, index + 1),
            None => (width, width),
        };
        out.push((kind, window[..emit_end].iter().collect::<String>()));
        start += next_start;
    }
}

#[cfg(test)]
mod wrap_tests {
    use super::*;

    fn wrapped_lines(input: &str, width: usize) -> Vec<String> {
        let mut out = Vec::new();
        wrap_word_aware(input, width, BlockKind::System, &mut out);
        out.into_iter().map(|(_, text)| text).collect()
    }

    #[test]
    fn wraps_on_word_boundaries_when_possible() {
        assert_eq!(
            wrapped_lines("alpha beta gamma", 8),
            vec!["alpha", "beta", "gamma"]
        );
    }

    #[test]
    fn splits_unbreakable_tokens_but_keeps_the_rest_whole() {
        let lines = wrapped_lines("abcdefghij klmno", 6);
        assert_eq!(lines, vec!["abcdef", "ghij", "klmno"]);
    }

    #[test]
    fn short_lines_pass_through_and_leading_space_never_loops() {
        assert_eq!(wrapped_lines("short", 80), vec!["short"]);
        assert_eq!(wrapped_lines("aaaaaaa bbb", 4), vec!["aaaa", "aaa", "bbb"]);
    }
}

impl Transcript {
    /// Jumps the viewport to the next line at/after the current top that
    /// contains `needle` (case-insensitive). Returns true when a match was
    /// found. Searching from the tail when following, so repeated searches
    /// walk upward through history.
    pub(crate) fn jump_to_match(&mut self, needle: &str, width: usize, height: usize) -> bool {
        let total = self.total_lines(width);
        if total == 0 || height == 0 {
            return false;
        }
        let needle = needle.to_lowercase();
        let lines = self.lines(width);
        let current_top = total
            .saturating_sub(height)
            .saturating_sub(self.scroll_up.min(total.saturating_sub(height)));
        // Walk downward from just above the current top; wrap once.
        for offset in 0..total {
            let idx = (current_top + offset) % total;
            if lines[idx].1.to_lowercase().contains(&needle) {
                let max_scroll = total.saturating_sub(height);
                self.scroll_up = (total - 1 - idx).min(max_scroll);
                return true;
            }
        }
        false
    }
}

impl Transcript {
    /// Lines containing `needle` (case-insensitive), for the find overlay.
    pub(crate) fn count_matches(&self, needle: &str, width: usize) -> usize {
        if needle.is_empty() {
            return 0;
        }
        let needle = needle.to_lowercase();
        self.lines(width)
            .iter()
            .filter(|(_, text)| text.to_lowercase().contains(&needle))
            .count()
    }
}

#[cfg(test)]
mod find_tests {
    use super::*;

    fn transcript() -> Transcript {
        let mut t = Transcript::default();
        t.push(BlockKind::User, "show me the orders table");
        t.push(BlockKind::Assistant, "Here is the orders summary.");
        t.push(BlockKind::Error, "column not found: ordrs");
        t
    }

    #[test]
    fn jump_finds_case_insensitive_and_reports_misses() {
        let mut t = transcript();
        assert!(t.jump_to_match("ORDERS", 80, 2));
        assert!(t.scroll_up > 0, "viewport moved to the match");
        assert!(!t.jump_to_match("nonexistent-needle", 80, 2));
        // Following-tail state is untouched by a miss.
        assert!(t.scroll_up > 0);
    }
}
