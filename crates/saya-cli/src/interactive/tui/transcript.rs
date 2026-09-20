use std::{cell::RefCell, rc::Rc};

pub(crate) mod chapters;
pub(crate) mod model;
pub(crate) mod view;

pub(crate) use model::{Block, BlockKind};
pub(crate) use view::rows;

pub(super) const MAX_BLOCKS: usize = 5000;
pub(super) const MAX_TOTAL_TEXT_BYTES: usize = 4 << 20;

pub(super) type WrapCache = RefCell<Option<(usize, Rc<rows::WrappedLines>)>>;

#[allow(dead_code)]
#[derive(Debug, Default)]
pub(crate) struct Transcript {
    pub(super) blocks: Vec<Block>,
    pub(super) scroll_up: usize,
    pub(super) cache: WrapCache,
    /// Folded finished chapters: pure view state, like `Block.group` — never
    /// persisted, never replayed, emptied by `clear()` with the blocks.
    pub(super) folded: std::collections::BTreeSet<u32>,
    /// Tool events buffered behind the shared grouper: the open run of
    /// `ToolRequested`/`ToolCompleted` pairs not yet closed by a boundary
    /// event. While the run is open its per-call lines also render live on
    /// the tail; the boundary flush folds them into one collapsed block.
    /// `None`'s and in-flight requests (`Some` with no completion yet) ride
    /// here only — never on a rendered block — so an interrupted stream
    /// leaves no half group behind.
    pub(super) pending_tools: Vec<model::PendingToolCall>,
}

#[allow(dead_code)]
impl Transcript {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(super) fn invalidate_cache(&self) {
        *self.cache.borrow_mut() = None;
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
