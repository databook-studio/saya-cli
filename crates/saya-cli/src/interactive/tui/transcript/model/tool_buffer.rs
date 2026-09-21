use saya_agent::ToolEffect;

use super::super::{Transcript, chapters};
use super::{Block, BlockKind, PendingToolCall};

impl Transcript {
    /// A run folds only when it is a multi-call all-ok group: two or more
    /// completed calls, every summary success-shaped. A single call renders as
    /// today; a group with a failure keeps the failure's full pair live. The
    /// failure half uses the grouper's own predicate on the displayed summary
    /// text (the transcript owns no `AgentEvent`s): the day the contract
    /// changes, both must move together.
    fn folds_run(completed: &[(String, serde_json::Value, Option<ToolEffect>, String)]) -> bool {
        completed.len() >= 2
            && completed
                .iter()
                .all(|(_, _, _, summary)| !crate::render::tool_groups::is_failure_summary(summary))
    }

    /// Buffers a tool request: mirrors today's per-call line onto the tail so
    /// the running stream stays legible, and holds the facts for the grouper.
    /// Through [`Transcript::mutate`] — anchors and counts like any append.
    pub(crate) fn buffer_tool_request(
        &mut self,
        name: String,
        arguments: serde_json::Value,
        effect: Option<ToolEffect>,
    ) {
        self.mutate(|t| {
            let chapter = chapters::chapter_for(&t.blocks, BlockKind::Tool);
            let before = t.blocks.len();
            for line in super::tool_lines::live_request_lines(&name, &arguments) {
                t.blocks.push(Block {
                    kind: BlockKind::Tool,
                    text: line,
                    chapter,
                    group: None,
                });
            }
            let live_blocks = t.blocks.len().saturating_sub(before);
            t.pending_tools.push(PendingToolCall {
                name,
                arguments,
                effect,
                summary: None,
                live_blocks,
                open: true,
            });
        });
    }

    /// Pairs a completion with its open request: mirrors the shared
    /// completion line onto the tail (`✓` for success, `✗` for failure).
    /// Returns false when no request is open — a stray completion the caller
    /// renders directly, outside any group. Through [`Transcript::mutate`],
    /// so the mark is on the very next render.
    pub(crate) fn buffer_tool_completion(&mut self, name: &str, summary: &str) -> bool {
        self.mutate(|t| {
            // Oldest open call of this name, not newest. The loop emits every
            // `ToolRequested` of a parallel batch first, then every
            // `ToolCompleted`, both in `assistant.tool_calls` order
            // (`turn_tools.rs`), so completions arrive in request order and must
            // pair FIFO. Matching newest-first swapped the summaries of two
            // same-name parallel calls, and the flushed group then showed one
            // call's arguments beside another call's result. Pairing is by name
            // only — there is no call id on the events — so FIFO is the strongest
            // correct rule available here.
            let Some(pending) = t
                .pending_tools
                .iter_mut()
                .find(|call| call.open && call.name == name)
            else {
                return false;
            };
            let chapter = chapters::chapter_for(&t.blocks, BlockKind::Tool);
            let before = t.blocks.len();
            t.blocks.push(Block {
                kind: BlockKind::Tool,
                text: crate::render::tool_groups::live_completion_line(name, summary),
                chapter,
                group: None,
            });
            pending.live_blocks += t.blocks.len().saturating_sub(before);
            pending.summary = Some(summary.to_owned());
            pending.open = false;
            true
        })
    }

    /// Drops the buffered run without rendering — the `TurnReset` retry path,
    /// through [`Transcript::mutate`]; removal only shrinks, so the anchor
    /// there is a no-op and the boundary is a plain invalidation.
    pub(crate) fn discard_tool_buffer(&mut self) {
        self.mutate(|t| {
            let live: usize = t.pending_tools.iter().map(|call| call.live_blocks).sum();
            for _ in 0..live {
                if t.blocks
                    .last()
                    .is_some_and(|b| b.kind == BlockKind::Tool && !b.is_collapsible())
                {
                    t.blocks.pop();
                } else {
                    break;
                }
            }
            t.pending_tools.clear();
        });
    }

    /// Folds the buffered run into blocks: completed calls shape through the
    /// shared grouper; a multi-call all-ok group lands as one collapsed block
    /// (the summary header with the per-call lines as view state), everything
    /// else keeps the live lines exactly as streamed. In-flight requests (no
    /// completion yet) keep their live lines and stay buffered: the boundary
    /// closed nothing for them. Through [`Transcript::mutate`] — folding only
    /// shrinks, so the anchor there is a no-op and the bookkeeping (bounds,
    /// invalidation) stays uniform. Nothing to fold is no mutation: no
    /// bookkeeping either.
    pub(crate) fn flush_tool_buffer(
        &mut self,
        request_lines: impl Fn(&str, &serde_json::Value) -> Vec<String>,
        completion_line: impl Fn(&str, &str) -> String,
    ) {
        if self.pending_tools.is_empty() {
            return;
        }
        self.mutate(|t| {
            let completed: Vec<(String, serde_json::Value, Option<ToolEffect>, String)> = t
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
                t.pending_tools.retain(|call| call.open);
                return;
            }
            let live: usize = t
                .pending_tools
                .iter()
                .filter(|call| !call.open)
                .map(|call| call.live_blocks)
                .sum();
            for _ in 0..live {
                if t.blocks
                    .last()
                    .is_some_and(|b| b.kind == BlockKind::Tool && !b.is_collapsible())
                {
                    t.blocks.pop();
                } else {
                    break;
                }
            }
            t.pending_tools.retain(|call| call.open);
            if completed.is_empty() {
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
                    folded.chapter = chapters::chapter_for(&t.blocks, BlockKind::Tool);
                    t.blocks.push(folded);
                    continue;
                }
                // Unreachable today: `is_collapsible_run` gates on exactly the
                // shape above, so every group here folds. The arm stays so a
                // future grouper change lands verbatim instead of vanishing.
                let chapter = chapters::chapter_for(&t.blocks, BlockKind::Tool);
                for line in shaped {
                    t.blocks.push(Block {
                        kind: BlockKind::Tool,
                        text: line,
                        chapter,
                        group: None,
                    });
                }
            }
        });
    }
}

#[cfg(test)]
#[path = "tool_buffer_tests.rs"]
mod parallel_pairing_tests;
