pub(crate) mod attempt;
pub(crate) mod mutation;
pub(crate) mod push;
pub(crate) mod tool_buffer;
pub(crate) mod tool_lines;

#[cfg(test)]
#[allow(unused_imports)]
pub(crate) use super::Transcript;
use super::chapters;

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

/// One buffered tool call: the request's facts for the grouper, the live
/// per-call lines, and whether the completion has arrived yet.
#[derive(Debug, Clone)]
pub(crate) struct PendingToolCall {
    pub(crate) name: String,
    pub(crate) arguments: serde_json::Value,
    pub(crate) effect: Option<saya_agent::ToolEffect>,
    pub(crate) summary: Option<String>,
    pub(crate) live_blocks: usize,
    pub(crate) open: bool,
}
