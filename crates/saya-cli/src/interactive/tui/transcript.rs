use rows::{Row, WrappedLines, wrap_word_aware};
use std::{cell::RefCell, rc::Rc};

use saya_agent::ToolEffect;

pub(crate) mod chapters;
pub(crate) mod rows;

const MAX_BLOCKS: usize = 5000;
const MAX_TOTAL_TEXT_BYTES: usize = 4 << 20;

type WrapCache = RefCell<Option<(usize, Rc<WrappedLines>)>>;

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BlockKind {
    User,
    Assistant,
    System,
    Error,
    Tool,
    /// A query result rendered as a box-drawing table. The block text is the
    /// full, untruncated table (what copy and persistence see); the view paints
    /// it with horizontal scrolling rather than word-wrapping, so a wide result
    /// stays readable.
    Table,
    /// The model's chain-of-thought, shown only when the user asked for it.
    /// Visually subordinate to the answer and excluded from clipboard copy and
    /// session persistence — reasoning restates database contents in prose.
    Thinking,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub(crate) struct Block {
    pub(crate) kind: BlockKind,
    pub(crate) text: String,
    /// Request chapter: 0 before any `User` block, +1 per `User` block.
    pub(crate) chapter: u32,
    /// A collapsed tool group renders as one header block; its per-call lines
    /// live here and render only while expanded. `None` on every other block.
    /// Pure view state on the block — never persisted, never replayed — so a
    /// resumed session never carries it.
    pub(crate) group: Option<ToolGroupView>,
}

/// The view state of one collapsed tool-call group: the header is the block
/// text (the `▸` summary the shared shaper emitted), the per-call `→` / `✓`
/// lines render in its place while `expanded`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ToolGroupView {
    pub(crate) expanded: bool,
    pub(crate) detail: Vec<String>,
    /// The `▾` header shown above the per-call lines while expanded.
    pub(crate) open_header: String,
}

impl Block {
    /// One block for a collapsed tool group: the summary header text with the
    /// per-call lines held as view state.
    pub(crate) fn tool_group(summary: String, detail: Vec<String>, open_header: String) -> Self {
        Self {
            kind: BlockKind::Tool,
            text: summary,
            chapter: chapters::PRE_CHAPTER,
            group: Some(ToolGroupView {
                expanded: false,
                detail,
                open_header,
            }),
        }
    }

    /// Whether this block is a collapsible (multi-call) tool group.
    pub(crate) fn is_collapsible(&self) -> bool {
        self.group.is_some()
    }
}

#[allow(dead_code)]
#[derive(Debug, Default)]
pub(crate) struct Transcript {
    blocks: Vec<Block>,
    scroll_up: usize,
    cache: WrapCache,
    /// Tool events buffered behind the shared grouper: the open run of
    /// `ToolRequested`/`ToolCompleted` pairs not yet closed by a boundary
    /// event. While the run is open its per-call lines also render live on
    /// the tail; the boundary flush folds them into one collapsed block.
    /// `None`'s and in-flight requests (`Some` with no completion yet) ride
    /// here only — never on a rendered block — so an interrupted stream
    /// leaves no half group behind.
    pending_tools: Vec<PendingToolCall>,
}

/// One buffered tool call: the request's facts for the grouper, the live
/// per-call lines, and whether the completion has arrived yet.
#[derive(Debug, Clone)]
struct PendingToolCall {
    name: String,
    arguments: serde_json::Value,
    effect: Option<ToolEffect>,
    summary: Option<String>,
    live_blocks: usize,
    open: bool,
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
        let chapter = chapters::chapter_for(&self.blocks, kind);
        let text = text.into();
        self.blocks.push(Block {
            kind,
            text,
            chapter,
            group: None,
        });
        self.enforce_bounds();
        self.invalidate_cache();
    }

    /// A run folds only when it is a multi-call all-ok group: two or more
    /// completed calls, every summary success-shaped. A single call renders as
    /// today; a group with a failure keeps the failure's full pair live. The
    /// failure half mirrors the grouper's contract directly instead of
    /// calling into it: the transcript owns no `AgentEvent`s, only names and
    /// summaries, so it re-checks the displayed summary text. The day the
    /// contract changes, both must move together.
    fn folds_run(completed: &[(String, serde_json::Value, Option<ToolEffect>, String)]) -> bool {
        completed.len() >= 2
            && completed
                .iter()
                .all(|(_, _, _, summary)| !summary.contains("failed"))
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

    /// Clears the text accumulated in the trailing block of `kind` — exactly
    /// the block the next [`append_delta`] of that kind would extend — so a
    /// re-streamed answer **replaces** what streamed so far instead of
    /// appending (the `TurnReset` retry path). No-op when the trailing block
    /// is not of `kind` (nothing streamed yet to discard).
    pub(crate) fn reset_delta(&mut self, kind: BlockKind) {
        if let Some(last) = self.blocks.last_mut().filter(|last| last.kind == kind) {
            last.text.clear();
            self.invalidate_cache();
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

    /// Buffers a tool request: mirrors today's per-call line onto the tail so
    /// the running stream stays legible, and holds the facts for the grouper.
    pub(crate) fn buffer_tool_request(
        &mut self,
        name: String,
        arguments: serde_json::Value,
        effect: Option<ToolEffect>,
    ) {
        let chapter = chapters::chapter_for(&self.blocks, BlockKind::Tool);
        let before = self.blocks.len();
        for line in Self::live_request_lines(&name, &arguments) {
            self.blocks.push(Block {
                kind: BlockKind::Tool,
                text: line,
                chapter,
                group: None,
            });
        }
        let live_blocks = self.blocks.len().saturating_sub(before);
        self.pending_tools.push(PendingToolCall {
            name,
            arguments,
            effect,
            summary: None,
            live_blocks,
            open: true,
        });
        self.enforce_bounds();
        self.invalidate_cache();
    }

    /// Pairs a completion with its open request: mirrors today's `✓` line onto
    /// the tail. Returns false when no request is open — a stray completion
    /// the caller renders directly, outside any group.
    pub(crate) fn buffer_tool_completion(&mut self, name: &str, summary: &str) -> bool {
        let Some(pending) = self
            .pending_tools
            .iter_mut()
            .rev()
            .find(|call| call.open && call.name == name)
        else {
            return false;
        };
        let chapter = chapters::chapter_for(&self.blocks, BlockKind::Tool);
        let before = self.blocks.len();
        self.blocks.push(Block {
            kind: BlockKind::Tool,
            text: format!("✓ {name}: {summary}"),
            chapter,
            group: None,
        });
        pending.live_blocks += self.blocks.len().saturating_sub(before);
        pending.summary = Some(summary.to_owned());
        pending.open = false;
        true
    }

    fn live_request_lines(name: &str, arguments: &serde_json::Value) -> Vec<String> {
        if let Some(call) = crate::agent::tools::sql_tool_call(name, arguments) {
            let header = match &call.target {
                Some(t) => format!("SQL · {t}"),
                None => "SQL".to_string(),
            };
            let body = call
                .sql
                .lines()
                .map(|l| format!("  {l}"))
                .collect::<Vec<_>>()
                .join("\n");
            return vec![format!("{header}\n{body}")];
        }
        vec![
            match crate::agent::tools::tool_call_detail(name, arguments) {
                Some(detail) => format!("→ {name}: {detail}"),
                None => format!("→ {name}"),
            },
        ]
    }

    /// Drops the buffered run without rendering — the `TurnReset` retry path.
    pub(crate) fn discard_tool_buffer(&mut self) {
        let live: usize = self.pending_tools.iter().map(|call| call.live_blocks).sum();
        for _ in 0..live {
            if self
                .blocks
                .last()
                .is_some_and(|block| block.kind == BlockKind::Tool && !block.is_collapsible())
            {
                self.blocks.pop();
            } else {
                break;
            }
        }
        self.pending_tools.clear();
        self.invalidate_cache();
    }

    /// Folds the buffered run into blocks: completed calls shape through the
    /// shared grouper; a multi-call all-ok group lands as one collapsed block
    /// (the summary header with the per-call lines as view state), everything
    /// else keeps the live lines exactly as streamed. In-flight requests (no
    /// completion yet) keep their live lines and stay buffered: the boundary
    /// closed nothing for them.
    pub(crate) fn flush_tool_buffer(
        &mut self,
        request_lines: impl Fn(&str, &serde_json::Value) -> Vec<String>,
        completion_line: impl Fn(&str, &str) -> String,
    ) {
        if self.pending_tools.is_empty() {
            return;
        }
        let completed: Vec<(String, serde_json::Value, Option<ToolEffect>, String)> = self
            .pending_tools
            .iter()
            .filter(|call| !call.open)
            .filter_map(|call| {
                call.summary.as_ref().map(|summary| {
                    (
                        call.name.clone(),
                        call.arguments.clone(),
                        call.effect,
                        summary.clone(),
                    )
                })
            })
            .collect();
        // Only a multi-call all-ok group folds: its live per-call lines pop
        // off and one collapsed block takes their place. A one-member group
        // or a group with a failure keeps the live lines exactly as streamed
        // — today's `→` / `✓` rendering, byte for byte, never the piped
        // text's `Using tool:` lines.
        if !Self::folds_run(&completed) {
            self.pending_tools.retain(|call| call.open);
            self.invalidate_cache();
            return;
        }
        let live: usize = self
            .pending_tools
            .iter()
            .filter(|call| !call.open)
            .map(|call| call.live_blocks)
            .sum();
        for _ in 0..live {
            if self
                .blocks
                .last()
                .is_some_and(|block| block.kind == BlockKind::Tool && !block.is_collapsible())
            {
                self.blocks.pop();
            } else {
                break;
            }
        }
        self.pending_tools.retain(|call| call.open);
        if completed.is_empty() {
            self.invalidate_cache();
            return;
        }
        let events: Vec<saya_agent::AgentEvent> = completed
            .iter()
            .flat_map(|(name, arguments, effect, summary)| {
                [
                    saya_agent::AgentEvent::ToolRequested {
                        name: name.clone(),
                        arguments: arguments.clone(),
                        effect: *effect,
                    },
                    saya_agent::AgentEvent::ToolCompleted {
                        name: name.clone(),
                        summary: summary.clone(),
                    },
                ]
            })
            .collect();
        let groups = crate::render::tool_groups::group_tool_events(&events);
        for group in &groups {
            let shaped = crate::render::tool_groups::shape_group(group);
            if group.calls.len() >= 2
                && shaped.len() == 1
                && !group.calls.iter().any(|call| call.failed)
            {
                let detail: Vec<String> = group
                    .calls
                    .iter()
                    .flat_map(|call| {
                        let mut lines = request_lines(&call.name, &call.arguments);
                        lines.push(completion_line(
                            &call.name,
                            call.summary.as_deref().unwrap_or(""),
                        ));
                        lines
                    })
                    .collect();
                let open_header = format!("▾{}", shaped[0].trim_start_matches('▸'));
                let mut folded = Block::tool_group(shaped[0].clone(), detail, open_header);
                folded.chapter = chapters::chapter_for(&self.blocks, BlockKind::Tool);
                self.blocks.push(folded);
                continue;
            }
            // Unreachable today: `is_collapsible_run` gates on exactly the
            // shape above, so every group here folds. The arm stays so a
            // future grouper change lands verbatim instead of vanishing.
            let chapter = chapters::chapter_for(&self.blocks, BlockKind::Tool);
            for line in shaped {
                self.blocks.push(Block {
                    kind: BlockKind::Tool,
                    text: line,
                    chapter,
                    group: None,
                });
            }
        }
        self.enforce_bounds();
        self.invalidate_cache();
    }

    fn lines(&self, width: usize) -> Rc<WrappedLines> {
        let eff = width.max(1);
        if let Some((_, lines)) = self.cache.borrow().as_ref().filter(|(w, _)| *w == eff) {
            return Rc::clone(lines);
        }
        let mut lines = Vec::new();
        for block in &self.blocks {
            if block.text.is_empty()
                && block
                    .group
                    .as_ref()
                    .filter(|group| group.expanded)
                    .is_none()
            {
                // Spacers (`push_spacer`'s `(System, "")`) stay visible but
                // bare: one empty body row, no label row — a label above every
                // blank line would be noise, and empty rows paint blank.
                lines.push(Row::body(block.kind, String::new()));
                continue;
            }
            // One label row per non-empty block, ahead of its body rows — so
            // every scroll/find metric derived from `lines()` counts the row
            // that paints, keeping "one entry per painted row" true.
            let mut labelled_yet = false;
            let mut emit_body = |raw: &str, lines: &mut WrappedLines| {
                if raw.is_empty() {
                    // Blank lines inside a block stay bare.
                    lines.push(Row::body(block.kind, String::new()));
                    return;
                }
                if !labelled_yet {
                    labelled_yet = true;
                    if let Some(label) = Row::label(block.kind) {
                        lines.push(label);
                    }
                }
                if block.kind == BlockKind::Table {
                    // A table row is one line of box drawing; word-wrapping it
                    // destroys the grid, so each line is kept whole and the
                    // view clips it horizontally instead.
                    lines.push(Row::body(block.kind, raw.to_string()));
                } else {
                    wrap_word_aware(raw, eff, block.kind, lines);
                }
            };
            if let Some(group) = block.group.as_ref().filter(|group| group.expanded) {
                for raw in std::iter::once(group.open_header.as_str())
                    .chain(group.detail.iter().map(String::as_str))
                {
                    emit_body(raw, &mut lines);
                }
                continue;
            }
            for raw in block.text.split('\n') {
                emit_body(raw, &mut lines);
            }
        }
        let rc = Rc::new(lines);
        *self.cache.borrow_mut() = Some((eff, Rc::clone(&rc)));
        rc
    }

    pub(crate) fn wrapped(&self, width: usize) -> WrappedLines {
        self.lines(width)
            .iter()
            .map(|row| Row {
                kind: row.kind,
                text: row.text.clone(),
                is_label: row.is_label,
            })
            .collect()
    }

    pub(crate) fn total_lines(&self, width: usize) -> usize {
        self.lines(width).len()
    }

    pub(crate) fn view(&self, width: usize, height: usize) -> WrappedLines {
        if height == 0 {
            return Vec::new();
        }
        // Every row `lines()` produces is a row that paints — label rows
        // included — so the tail view windows over all rows and
        // `total_lines` equals the painted row count again.
        let painted: WrappedLines = self
            .lines(width)
            .iter()
            .map(|row| Row {
                kind: row.kind,
                text: row.text.clone(),
                is_label: row.is_label,
            })
            .collect();
        let rem = painted.len().saturating_sub(height);
        if rem == 0 {
            return painted;
        }
        let start = rem - self.scroll_up.min(rem);
        painted[start..start + height].to_vec()
    }

    /// Like [`view`], but table blocks are painted through the wide-table
    /// view: each table line is horizontally clipped (and optionally
    /// column-filtered) to `width` instead of left whole. The line count is
    /// unchanged, so vertical scroll metrics from [`total_lines`] still match.
    /// The offset/column state is passed in from the view — it never lives on
    /// the transcript data.
    pub(crate) fn wide_view(
        &self,
        width: usize,
        height: usize,
        wv: &super::types::WideTableView,
    ) -> WrappedLines {
        let src = self.lines(width);
        let mut full: WrappedLines = Vec::with_capacity(src.len());
        let mut i = 0;
        while i < src.len() {
            if src[i].kind == BlockKind::Table && !src[i].is_label {
                // A table block's lines are contiguous; collect the run, then
                // split it into individual tables (each starts with ┌) so two
                // adjacent results are clipped independently. The label row
                // opens the run and passes through untouched.
                let run_start = i;
                while i < src.len() && src[i].kind == BlockKind::Table {
                    i += 1;
                }
                let run = src[run_start..i]
                    .iter()
                    .map(|row| row.text.clone())
                    .collect::<Vec<_>>();
                // A run of exactly one line is the lone label row (a
                // non-empty table block is label + grid lines): it passes
                // through untouched, never through the grid clipper.
                if run.len() == 1 && src[run_start].is_label {
                    full.push(Row {
                        kind: BlockKind::Table,
                        text: run[0].clone(),
                        is_label: true,
                    });
                    continue;
                }
                // The run opens with the label row, ahead of the grid lines
                // the clipper expects — clip the grid, keep the label
                // verbatim, and the line count is unchanged.
                let (label, grid) = match src[run_start].is_label {
                    true => (Some(&src[run_start]), &run[1..]),
                    false => (None, &run[..]),
                };
                let clipped = super::table::clip_table_block(grid, wv, width);
                debug_assert_eq!(clipped.len(), grid.len());
                if let Some(label) = label {
                    full.push(Row {
                        kind: BlockKind::Table,
                        text: label.text.clone(),
                        is_label: true,
                    });
                }
                for line in clipped {
                    full.push(Row::body(BlockKind::Table, line));
                }
            } else {
                full.push(Row {
                    kind: src[i].kind,
                    text: src[i].text.clone(),
                    is_label: src[i].is_label,
                });
                i += 1;
            }
        }
        // Every row `lines()` produces is a row that paints — label rows
        // included — so the tail view windows over the full rows and
        // `total_lines` equals the painted row count again.
        let out: WrappedLines = full;
        if height == 0 {
            return Vec::new();
        }
        let rem = out.len().saturating_sub(height);
        if rem == 0 {
            return out;
        }
        let start = rem - self.scroll_up.min(rem);
        out[start..start + height].to_vec()
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

    /// Toggles the most recent collapsible tool group between its one-line
    /// summary and its full per-call sequence. Returns true when a group was
    /// toggled. There is no per-block cursor on the transcript, so this is the
    /// smallest honest affordance: the newest group is the one the user just
    /// watched stream in. Newest-only is the decided affordance, not an
    /// unfinished one: the boundary rule stands and per-block cursor is
    /// deliberately not built. Returns false (no-op) when no group exists; nothing
    /// is pushed either way.
    pub(crate) fn toggle_latest_group(&mut self) -> bool {
        let toggled = self
            .blocks
            .iter_mut()
            .rev()
            .find(|block| block.is_collapsible())
            .map(|block| {
                let group = block.group.as_mut().expect("found by the predicate");
                group.expanded = !group.expanded;
            })
            .is_some();
        if toggled {
            self.invalidate_cache();
        }
        toggled
    }
}

#[cfg(test)]
mod label_row_red_tests {
    // RED: these tests name the `Row` API (`row.text`, `row.is_label`) that
    // does not exist yet — `WrappedLines` is still `Vec<(BlockKind, String)>`,
    // so this module fails to compile until `rows.rs` lands.
    use super::*;

    fn texts(rows: &WrappedLines) -> Vec<&str> {
        rows.iter().map(|row| row.text.as_str()).collect()
    }

    #[test]
    fn every_non_empty_block_gains_exactly_one_label_row() {
        let mut t = Transcript::new();
        t.push(BlockKind::User, "hello");
        t.push(BlockKind::Assistant, "hi");
        let rows = t.wrapped(80);
        assert_eq!(rows.len(), 4, "two bodies plus two labels: {rows:?}");
        assert!(rows[0].is_label && rows[0].text == "YOU");
        assert!(!rows[1].is_label && rows[1].text == "hello");
        assert!(rows[2].is_label && rows[2].text == "SAYA");
        assert!(!rows[3].is_label && rows[3].text == "hi");
    }

    #[test]
    fn an_empty_block_gains_no_label_row() {
        let mut t = Transcript::new();
        t.push(BlockKind::System, "");
        let rows = t.wrapped(80);
        assert_eq!(rows.len(), 1, "a spacer keeps its bare empty row");
        assert!(
            rows.iter().all(|row| !row.is_label),
            "but gains no label row: {rows:?}"
        );
    }

    #[test]
    fn find_never_lands_on_a_label_row() {
        let mut t = Transcript::new();
        t.push(BlockKind::User, "nothing to match here");
        assert_eq!(
            t.count_matches("saya", 80),
            0,
            "the only 'saya' is the SAYA label row"
        );
    }

    #[test]
    fn consecutive_same_kind_blocks_each_get_their_own_label() {
        let mut t = Transcript::new();
        t.push(BlockKind::User, "first");
        t.push(BlockKind::User, "second");
        let rows = t.wrapped(80);
        let labels: Vec<&str> = rows
            .iter()
            .filter(|row| row.is_label)
            .map(|row| row.text.as_str())
            .collect();
        assert_eq!(labels, vec!["YOU", "YOU"]);
    }

    #[test]
    fn total_lines_still_equals_the_rows_produced() {
        let mut t = Transcript::new();
        t.push(BlockKind::User, "hello");
        t.push(BlockKind::Assistant, "hi");
        let rows = t.wrapped(80);
        assert_eq!(t.total_lines(80), texts(&rows).len());
        assert_eq!(t.total_lines(80), 4);
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
        let s: String = t2.wrapped(10).iter().map(|row| row.text.as_str()).collect();
        assert_eq!(s, "1234567890abcdefghij");

        let (mut t3, mut t4) = (Transcript::new(), Transcript::new());
        t3.push(BlockKind::Error, "a\n\nb\n");
        t4.push(BlockKind::Tool, "日日日日日");
        assert!(t3.wrapped(10).len() == 4 && t4.wrapped(2).len() == 4);

        t.clear();
        t.push(BlockKind::User, "l1\nl2\nl3\nl4\nl5");
        let texts = |v: WrappedLines| v.into_iter().map(|row| row.text).collect::<Vec<_>>();
        assert_eq!(texts(t.view(10, 3)), ["l3", "l4", "l5"]);
        t.scroll_up(1, 10, 3);
        assert_eq!(texts(t.view(10, 3)), ["l2", "l3", "l4"]);
        t.scroll_up(100, 10, 3);
        assert_eq!(texts(t.view(10, 3)), ["YOU", "l1", "l2"]);
        t.scroll_down(1);
        assert_eq!(t.view(10, 3)[0].text, "l1");
        t.scroll_down(10);
        assert_eq!(t.view(10, 3)[0].text, "l3");
        t.scroll_up(2, 10, 3);
        t.scroll_to_bottom();
        assert_eq!(t.view(10, 3)[0].text, "l3");

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
        assert!(Rc::ptr_eq(&l1, &l2) && tc.wrapped(10).len() == 3);

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
        assert_eq!(tb2.view(10, 1)[0].kind, BlockKind::Assistant);
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
        assert_eq!(wrapped.len(), MAX_BLOCKS * 2);
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
            if lines[idx].is_label {
                continue;
            }
            if lines[idx].text.to_lowercase().contains(&needle) {
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
            .filter(|row| !row.is_label && row.text.to_lowercase().contains(&needle))
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
