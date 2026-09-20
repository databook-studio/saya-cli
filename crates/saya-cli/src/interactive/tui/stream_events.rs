//! Maps streamed agent events onto transcript blocks.

use super::table;
use super::transcript::{BlockKind, Transcript};
use saya_agent::AgentEvent;

/// Applies one streamed agent event to the transcript.
///
/// `show_thinking` gates whether the model's chain-of-thought reaches the
/// transcript: off by default, so a user who did not ask for it never sees it.
/// When on, reasoning is pushed as a dimmed `Thinking` block — visually
/// subordinate to the answer, never mistakable for it. Either way reasoning is
/// in-memory only and never persisted.
///
/// Tool events buffer behind the shared grouper and flush at the next boundary
/// event: a run of tool calls lands as one collapsed block carrying the
/// Decision-2 summary (the same string the piped surface emits), expandable to
/// today's per-call `→` / `✓` lines. Streaming with the tail followed shows the
/// per-call lines as they arrive (a collapse imposed mid-stream would rewrite
/// history the user just watched); the group collapses when the boundary event
/// that ends it arrives.
pub(crate) fn apply_event(transcript: &mut Transcript, event: AgentEvent, show_thinking: bool) {
    // A caller that ends the stream after a tool run (tests, the panel's
    // final `Complete`) flushes on the boundary below. A caller that stops
    // mid-run with no boundary leaves buffered calls unrendered, so a trailing
    // flush would misattribute the next turn's text as this group's boundary —
    // keep the buffer, don't flush it here.
    if is_group_member(&event) {
        if let Some(other) = buffer_tool_event(transcript, event) {
            apply_boundary_event(transcript, other, show_thinking);
        }
        return;
    }
    // The retry discards the run in flight: the failure it reports belongs to
    // the transport, never to a collapsed summary.
    if matches!(event, AgentEvent::TurnReset) {
        transcript.discard_tool_buffer();
    } else {
        flush_tool_buffer(transcript);
    }
    apply_boundary_event(transcript, event, show_thinking);
}

/// Buffers one member event and, while the group is still open, mirrors the
/// per-call line onto the tail block so the running stream stays legible.
/// Returns a boundary event to re-dispatch when the stream's legibility and
/// the buffer disagree (never today: the open group always shows calls live).
fn is_group_member(event: &AgentEvent) -> bool {
    matches!(
        event,
        AgentEvent::ToolRequested { .. } | AgentEvent::ToolCompleted { .. }
    )
}

/// Mirrors one member event onto the tail as today's per-call line (so the
/// running stream stays legible) and buffers the call's facts for the grouper.
/// `TurnReset` discards the buffer: it retries the turn, never completes a run.
///
/// Returns a stray completion that arrived with no open request: it renders as
/// today's `✓` line and stays out of the buffer, so it can never join a group.
fn buffer_tool_event(transcript: &mut Transcript, event: AgentEvent) -> Option<AgentEvent> {
    match event {
        AgentEvent::ToolRequested {
            name,
            arguments,
            effect,
        } => {
            transcript.buffer_tool_request(name, arguments, effect);
            None
        }
        AgentEvent::ToolCompleted { name, summary } => {
            if transcript.buffer_tool_completion(&name, &summary) {
                None
            } else {
                // Stray completion: no open request to pair it with. Render
                // today's line and keep it out of the group.
                transcript.push(BlockKind::Tool, format!("✓ {name}: {summary}"));
                None
            }
        }
        other => Some(other),
    }
}

/// Folds the buffered run into blocks at the boundary: one collapsed block
/// for a multi-call group, today's verbatim lines for a single call or a
/// group with a failure.
fn flush_tool_buffer(transcript: &mut Transcript) {
    transcript.flush_tool_buffer(request_lines, |name, summary| {
        format!("✓ {name}: {summary}")
    });
}

/// Today's verbatim request rendering, one block per line: the SQL block or
/// the `→` line, exactly as the TUI rendered before this slice.
fn request_lines(name: &str, arguments: &serde_json::Value) -> Vec<String> {
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

fn apply_boundary_event(transcript: &mut Transcript, event: AgentEvent, show_thinking: bool) {
    match event {
        AgentEvent::AssistantText { text } => {
            if !matches!(
                transcript.blocks().last().map(|b| b.kind),
                Some(BlockKind::Assistant)
            ) {
                // first chunk of the answer: separate it from the tool/SQL lines above
                if transcript
                    .blocks()
                    .last()
                    .is_some_and(|b| !b.text.is_empty())
                {
                    transcript.push(BlockKind::System, String::new());
                }
            }
            transcript.append_delta(BlockKind::Assistant, &text);
        }
        // The provider stream failed mid-answer and the loop is retrying the
        // turn. The text streamed so far is discarded: clear the trailing
        // assistant block (the one the next delta would extend) so the
        // re-streamed answer replaces it instead of appending to it.
        AgentEvent::TurnReset => transcript.reset_delta(BlockKind::Assistant),
        AgentEvent::ToolRequested {
            name, arguments, ..
        } => {
            if let Some(call) = crate::agent::tools::sql_tool_call(&name, &arguments) {
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
                let text = format!("{header}\n{body}");
                transcript.push(BlockKind::Tool, text);
            } else {
                let line = match crate::agent::tools::tool_call_detail(&name, &arguments) {
                    Some(detail) => format!("→ {name}: {detail}"),
                    None => format!("→ {name}"),
                };
                transcript.push(BlockKind::Tool, line);
            }
        }
        AgentEvent::ToolCompleted { name, summary } => {
            transcript.push(BlockKind::Tool, format!("✓ {name}: {summary}"))
        }
        AgentEvent::ToolDenied { name, reason } => {
            transcript.push(BlockKind::System, format!("✗ {name} denied: {reason}"))
        }
        // What memory supplied, shown before the answer streams. The shared
        // shaper centralizes the wording; an empty
        // result (Ran-and-found-nothing) is silence — push nothing.
        AgentEvent::KnowledgeSupplied {
            outcome,
            contracts,
            dropped_by_bounds,
        } => {
            let text =
                crate::render::knowledge_supplied_text(outcome, &contracts, dropped_by_bounds);
            if !text.is_empty() {
                // Strip the trailing newline: the transcript stores line text
                // without a delimiter and re-wraps per line; a trailing '\n' would
                // push a blank line into the block.
                transcript.push(BlockKind::System, text.trim_end_matches('\n'));
            }
        }
        // A confirmed claim the turn's SQL contradicted. Trails the
        // answer — emitted after the loop — so a System block pushed here lands
        // below the assistant text, where a "the SQL contradicted a confirmed
        // claim" notice belongs. The shared shaper centralizes the wording; an
        // empty finding set is silence (the runtime emits nothing, but this
        // guards a directly-constructed event too).
        AgentEvent::KnowledgeOverridden { findings } => {
            let text = crate::render::knowledge_overridden_text(&findings);
            if !text.is_empty() {
                transcript.push(BlockKind::System, text.trim_end_matches('\n'));
            }
        }
        // Extraction timed out or errored after the turn succeeded (spec
        // packet-54). Trails the answer — emitted after the loop — so a System
        // block pushed here lands below the assistant text, where "and I did
        // not learn from this turn" belongs. Shares the shaper with the
        // headless path so the wording lives in one place; the line is never
        // empty for a known reason, so the block always pushes.
        AgentEvent::KnowledgeLearningSkipped { reason } => {
            let text = crate::render::learning_skipped_text(reason);
            if !text.is_empty() {
                transcript.push(BlockKind::System, text.trim_end_matches('\n'));
            }
        }
        // One fact learned this turn. Trails the answer — the runtime emits it
        // after the loop — so it lands below the assistant text, where "and I
        // kept this" belongs. Shares the shaper with the headless path; an
        // undescribable claim is silence, never a raw token.
        AgentEvent::KnowledgeProposed { claim } => {
            let text = crate::render::knowledge_learned_text(&claim);
            if !text.is_empty() {
                transcript.push(BlockKind::System, text.trim_end_matches('\n'));
            }
        }
        // The model's chain-of-thought. Shown only when the user opted in; otherwise
        // accepted and dropped, so the event never reaches the catch-all and never
        // renders as an error. When shown it lands as a dimmed `Thinking` block,
        // separate from the assistant answer and visually subordinate to it. Reasoning
        // is in-memory only: the transcript is never serialized, and the persisted
        // session types carry role + content only, so holding it here cannot reach a
        // session file regardless of the display toggle.
        AgentEvent::ReasoningText { text } => {
            if show_thinking && !text.is_empty() {
                transcript.push(BlockKind::Thinking, text);
            }
        }
        // The token counts one provider call reported. Accepted and dropped:
        // the transcript already shows a per-turn token line from the run
        // output at `Done`, and `/usage` breaks the session down, so a block
        // here would duplicate them. The event exists for the JSON/NDJSON
        // boundary; it must not reach the catch-all and disappear silently
        // into an error.
        AgentEvent::Usage { .. } => {}
        AgentEvent::Complete => {
            transcript.reformat_last(BlockKind::Assistant, table::format_markdown_tables);
        }
        // Silent bookkeeping the grouper must still treat as a boundary.
        // `Usage` arrives after the answer finished streaming and carries no
        // content — but a group cannot span it, exactly as the piped adapter
        // flushes on every non-member event including silent ones.
        AgentEvent::KnowledgeLearningStarted => {}
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use saya_agent::{
        KnowledgeOutcome, LearningSkipReason, OverrideFindingDto, SuppliedClaimDto,
        SuppliedContractDto,
    };
    use saya_types::{ClaimId, ClaimStatus};

    fn dto_claim(id: &str, kind: &str, value: &str, status: ClaimStatus) -> SuppliedClaimDto {
        SuppliedClaimDto {
            claim_id: ClaimId::parse(id).unwrap(),
            kind: kind.into(),
            value: value.into(),
            column: None,
            status,
        }
    }

    fn dto_contract(claims: Vec<SuppliedClaimDto>) -> SuppliedContractDto {
        SuppliedContractDto {
            profile: "analytics".into(),
            object: "catalog.public.orders".into(),
            schema_state: "current".into(),
            claims,
        }
    }

    fn last_block_text(transcript: &Transcript) -> Option<&str> {
        transcript.blocks().last().map(|b| b.text.as_str())
    }

    fn write_effect() -> Option<saya_agent::ToolEffect> {
        Some(saya_agent::ToolEffect {
            database_data: false,
            external_side_effect: false,
            requires_approval: false,
            local_state: saya_agent::LocalStateEffect::WriteWorkspace,
        })
    }

    /// C3 property 1: a collapsed group renders one block whose text equals
    /// the shaper's output — the same string the piped surface would emit, so
    /// the two surfaces cannot drift.
    #[test]
    fn collapsed_group_renders_one_block_equal_to_the_shaper_output() {
        let events = two_write_calls();
        let groups = crate::render::tool_groups::group_tool_events(&events);
        assert_eq!(groups.len(), 1, "precondition: one run is one group");
        let shaped = crate::render::tool_groups::shape_group(&groups[0]);
        assert_eq!(shaped.len(), 1, "precondition: the shaper collapses it");

        let mut transcript = Transcript::new();
        for event in events {
            apply_event(&mut transcript, event, false);
        }
        apply_event(&mut transcript, AgentEvent::complete(), false);
        let texts: Vec<&str> = transcript
            .blocks()
            .iter()
            .map(|b| b.text.as_str())
            .collect();
        assert_eq!(
            texts,
            vec![shaped[0].as_str()],
            "a collapsed group must render one block equal to the shaper's output"
        );
    }

    /// C3 property 2: toggling expands to the per-call `→` / `✓` lines and
    /// toggling again collapses back to the one-line summary.
    #[test]
    fn toggling_expands_to_the_per_call_lines_and_back() {
        let mut transcript = Transcript::new();
        for event in two_write_calls() {
            apply_event(&mut transcript, event, false);
        }
        apply_event(&mut transcript, AgentEvent::complete(), false);
        assert_eq!(transcript.blocks().len(), 1, "collapsed to one block");

        assert!(
            transcript.toggle_latest_group(),
            "a group is there to expand"
        );
        let wrapped = transcript.wrapped(200);
        let shown: Vec<&str> = wrapped
            .iter()
            .filter(|row| !row.is_label)
            .map(|row| row.text.as_str())
            .collect();
        assert_eq!(
            shown,
            vec![
                "▾ 2 tool calls · ok — workspace_write notes.md, other.md",
                "→ workspace_write: notes.md",
                "✓ workspace_write: notes.md written",
                "→ workspace_write: other.md",
                "✓ workspace_write: other.md written",
            ],
            "expanded shows today's per-call lines verbatim under a ▾ header"
        );

        assert!(transcript.toggle_latest_group(), "toggling again collapses");
        let wrapped = transcript.wrapped(200);
        let shown: Vec<&str> = wrapped
            .iter()
            .filter(|row| !row.is_label)
            .map(|row| row.text.as_str())
            .collect();
        assert_eq!(
            shown,
            vec!["▸ 2 tool calls · ok — workspace_write notes.md, other.md"],
            "collapsed again to the one-line summary"
        );
    }

    /// C3 property 3: expansion survives a re-render and a newly streamed
    /// event — it is view state on the block, not a render-time flag.
    #[test]
    fn expansion_survives_rerender_and_a_newly_streamed_event() {
        let mut transcript = Transcript::new();
        for event in two_write_calls() {
            apply_event(&mut transcript, event, false);
        }
        apply_event(&mut transcript, AgentEvent::complete(), false);
        assert!(transcript.toggle_latest_group(), "expand the group");

        // A re-render: the same wrapped lines, still expanded.
        let first: Vec<String> = transcript
            .wrapped(200)
            .iter()
            .filter(|row| !row.is_label)
            .map(|row| row.text.clone())
            .collect();
        let second: Vec<String> = transcript
            .wrapped(200)
            .iter()
            .filter(|row| !row.is_label)
            .map(|row| row.text.clone())
            .collect();
        assert_eq!(first, second, "re-render must not reset expansion");
        assert!(first[0].starts_with('▾'), "still expanded: {first:?}");

        // A newly streamed event lands after the group without resetting it.
        apply_event(&mut transcript, AgentEvent::assistant_text("done"), false);
        let wrapped = transcript.wrapped(200);
        let shown: Vec<&str> = wrapped
            .iter()
            .filter(|row| !row.is_label)
            .map(|row| row.text.as_str())
            .collect();
        assert!(shown[0].starts_with('▾'), "expansion survives: {shown:?}");
        assert_eq!(
            shown.last(),
            Some(&"done"),
            "the new event lands: {shown:?}"
        );
    }

    /// C3 property 4: a one-member group renders exactly as today — the same
    /// blocks, byte for byte — with no toggle affordance.
    #[test]
    fn one_member_group_renders_as_today_with_no_toggle() {
        let events = vec![
            AgentEvent::tool_requested(
                "workspace_write",
                serde_json::json!({"path": "notes.md", "content": "hi"}),
                write_effect(),
            ),
            AgentEvent::ToolCompleted {
                name: "workspace_write".into(),
                summary: "notes.md written".into(),
            },
        ];
        let mut grouped = Transcript::new();
        for event in events {
            apply_event(&mut grouped, event, false);
        }
        apply_event(&mut grouped, AgentEvent::complete(), false);

        assert_eq!(grouped.blocks().len(), 2, "two per-call blocks, not one");
        assert_eq!(grouped.blocks()[0].text, "→ workspace_write: notes.md");
        assert_eq!(
            grouped.blocks()[1].text,
            "✓ workspace_write: notes.md written"
        );
        assert!(
            grouped.blocks().iter().all(|b| !b.is_collapsible()),
            "no block may offer a toggle"
        );
        assert!(
            !grouped.toggle_latest_group(),
            "toggling with no group is a no-op"
        );
        assert_eq!(grouped.blocks().len(), 2, "the no-op toggled nothing");
    }

    /// C3 property 5: a group with a failure is not collapsible into a count —
    /// the failure's full pair renders as it does today: today's `→` / `✓`
    /// lines, never the piped text's `Using tool:` rendering.
    #[test]
    fn group_with_a_failure_renders_the_full_pair() {
        let events = vec![
            AgentEvent::tool_requested(
                "workspace_write",
                serde_json::json!({"path": "notes.md", "content": "hi"}),
                write_effect(),
            ),
            AgentEvent::ToolCompleted {
                name: "workspace_write".into(),
                summary: "notes.md written".into(),
            },
            AgentEvent::tool_requested(
                "run_command",
                serde_json::json!({"program": "pytest", "args": ["-q"]}),
                Some(saya_agent::ToolEffect {
                    database_data: false,
                    external_side_effect: true,
                    requires_approval: false,
                    local_state: saya_agent::LocalStateEffect::WriteWorkspace,
                }),
            ),
            AgentEvent::ToolCompleted {
                name: "run_command".into(),
                summary: "failed pytest".into(),
            },
        ];
        let groups = crate::render::tool_groups::group_tool_events(&events);
        assert_eq!(groups.len(), 1, "precondition: one run is one group");
        assert!(
            groups[0].calls.iter().any(|call| call.failed),
            "precondition: the group carries a failure"
        );

        let mut transcript = Transcript::new();
        for event in events {
            apply_event(&mut transcript, event, false);
        }
        apply_event(&mut transcript, AgentEvent::complete(), false);

        let texts: Vec<&str> = transcript
            .blocks()
            .iter()
            .map(|b| b.text.as_str())
            .collect();
        assert_eq!(
            texts,
            vec![
                "→ workspace_write: notes.md",
                "✓ workspace_write: notes.md written",
                "→ run_command: pytest",
                "✓ run_command: failed pytest",
            ],
            "the failure's full pair renders as today, successes uncollapsed"
        );
        assert!(
            transcript.blocks().iter().all(|b| !b.is_collapsible()),
            "a group with a failure offers no toggle"
        );
        assert!(
            texts.iter().any(|line| line.contains("failed pytest")),
            "the failure text is visible: {texts:?}"
        );
    }

    /// C3 property 6: expansion state is never written to the session file,
    /// and a resumed session does not carry it — replay renders through
    /// `replay.rs`, unchanged, with no group state. (Why: the transcript is
    /// never serialized — `SessionState` carries role + content only — so the
    /// strongest check available is the serialized session plus the replay
    /// path both showing no group text.)
    #[test]
    fn expansion_state_is_not_persisted_and_resume_carries_none_of_it() {
        use crate::interactive::session_state::SessionState;

        let mut transcript = Transcript::new();
        for event in two_write_calls() {
            apply_event(&mut transcript, event, false);
        }
        apply_event(&mut transcript, AgentEvent::complete(), false);
        assert!(transcript.toggle_latest_group(), "expand the group");

        let mut session =
            SessionState::new("s1", Some(String::from("analytics")), String::from("m"));
        session.record_turn("do the writes", "wrote both files", false, Vec::new());
        let json = serde_json::to_string(&session).expect("serializes");
        assert!(
            !json.contains("expanded"),
            "expansion state leaked into the session file: {json}"
        );

        let replayed = crate::interactive::tui::replay::history_blocks(&session);
        assert!(
            replayed.iter().all(|(_, text)| !text.starts_with('▸')),
            "replay renders through replay.rs, not the grouper: {replayed:?}"
        );
    }

    /// Extra: while the group is still streaming, the tail shows the per-call
    /// lines live — collapsing mid-stream would rewrite history the user just
    /// watched. (Why: the boundary rule says a group closes at the next
    /// content event; until then the calls are still arriving.)
    #[test]
    fn open_group_shows_per_call_lines_live_until_the_boundary() {
        let mut transcript = Transcript::new();
        for event in two_write_calls() {
            apply_event(&mut transcript, event, false);
        }
        let texts: Vec<&str> = transcript
            .blocks()
            .iter()
            .map(|b| b.text.as_str())
            .collect();
        assert_eq!(
            texts,
            vec![
                "→ workspace_write: notes.md",
                "✓ workspace_write: notes.md written",
                "→ workspace_write: other.md",
                "✓ workspace_write: other.md written",
            ],
            "before the boundary the stream shows today's lines live"
        );
        apply_event(&mut transcript, AgentEvent::assistant_text("done"), false);
        let texts: Vec<&str> = transcript
            .blocks()
            .iter()
            .map(|b| b.text.as_str())
            .collect();
        assert_eq!(
            texts,
            vec![
                "▸ 2 tool calls · ok — workspace_write notes.md, other.md",
                "",
                "done",
            ],
            "the boundary collapses the run and the text follows"
        );
    }

    /// Property 2 (TUI half): `approvals_are_untouched` — the denial is a
    /// boundary that splits the surrounding calls into one-member groups
    /// (today's `→` / `✓` lines, no toggle), and the denial itself renders
    /// as today's `✗` line. The key half of the property lives beside the
    /// keys (`keys::approval_modal_tests::grouping_leaves_approval_keys_untouched`):
    /// Enter still toggles a group and never approves. Prompt bytes and the
    /// bypass line live beside their seams (see the piped half for the
    /// pointers); the transcript path never renders either string.
    #[test]
    fn approvals_are_untouched() {
        let mut transcript = Transcript::new();
        apply_event(
            &mut transcript,
            AgentEvent::tool_requested(
                "workspace_write",
                serde_json::json!({"path": "a.md", "content": "hi"}),
                write_effect(),
            ),
            false,
        );
        apply_event(
            &mut transcript,
            AgentEvent::ToolCompleted {
                name: "workspace_write".into(),
                summary: "a.md written".into(),
            },
            false,
        );
        apply_event(
            &mut transcript,
            AgentEvent::ToolDenied {
                name: "run_command".into(),
                reason: "denied".into(),
            },
            false,
        );
        let texts: Vec<&str> = transcript
            .blocks()
            .iter()
            .map(|b| b.text.as_str())
            .collect();
        assert_eq!(
            texts,
            vec![
                "→ workspace_write: a.md",
                "✓ workspace_write: a.md written",
                "✗ run_command denied: denied",
            ],
            "one-member groups stay verbatim and the denial is its own line"
        );
        assert!(
            transcript.blocks().iter().all(|b| !b.is_collapsible()),
            "the denial splits the run: no collapsible group may span it"
        );
    }

    /// Two successful workspace writes: the shared fixture every C3 property
    /// builds its group from.
    fn two_write_calls() -> Vec<AgentEvent> {
        vec![
            AgentEvent::tool_requested(
                "workspace_write",
                serde_json::json!({"path": "notes.md", "content": "hi"}),
                write_effect(),
            ),
            AgentEvent::ToolCompleted {
                name: "workspace_write".into(),
                summary: "notes.md written".into(),
            },
            AgentEvent::tool_requested(
                "workspace_write",
                serde_json::json!({"path": "other.md", "content": "hi"}),
                write_effect(),
            ),
            AgentEvent::ToolCompleted {
                name: "workspace_write".into(),
                summary: "other.md written".into(),
            },
        ]
    }

    /// C0 property 4 (TUI half): the `→` request block names the same file
    /// the piped half pins beside the shared seam — both adapters render
    /// from `tool_call_detail`, so they cannot drift.
    #[test]
    fn tui_request_block_names_the_same_file_as_the_piped_line() {
        use saya_agent::{LocalStateEffect, ToolEffect};

        let mut transcript = Transcript::new();
        apply_event(
            &mut transcript,
            AgentEvent::tool_requested(
                "workspace_write",
                serde_json::json!({"path": "notes.md", "content": "hi"}),
                Some(ToolEffect {
                    database_data: false,
                    external_side_effect: false,
                    requires_approval: false,
                    local_state: LocalStateEffect::WriteWorkspace,
                }),
            ),
            false,
        );
        let block = last_block_text(&transcript).expect("a block was pushed");
        assert!(
            block.contains("notes.md"),
            "the TUI request block must name the file: {block:?}"
        );
    }

    /// KnowledgeSupplied pushes a System block whose text names the supplied
    /// claims. Asserts on the rendered transcript, not state.
    #[test]
    fn knowledge_supplied_pushes_a_system_block_with_the_claims() {
        let mut t = Transcript::new();
        apply_event(
            &mut t,
            AgentEvent::knowledge_supplied(
                KnowledgeOutcome::Ran {
                    store_unavailable: false,
                },
                vec![dto_contract(vec![
                    dto_claim("c-1", "table_alias", "orders", ClaimStatus::Confirmed),
                    dto_claim(
                        "c-2",
                        "default_time_column",
                        "created_at",
                        ClaimStatus::Candidate,
                    ),
                ])],
                0,
            ),
            false,
        );
        let block = last_block_text(&t).expect("a block was pushed");
        // The header points at /queue, the action the learn
        // path already names, beside the unconfirmed count it always carried.
        assert!(
            block.starts_with("memory supplied · 2 claims (1 unconfirmed — review with /queue)"),
            "{block}"
        );
        assert!(block.contains("table_alias  orders  confirmed"), "{block}");
        // The candidate is marked on its line.
        assert!(block.contains("candidate  (unconfirmed)"), "{block}");
        // The block is a System block, not a Tool block.
        assert_eq!(t.blocks().last().unwrap().kind, BlockKind::System);
    }

    /// The three outcomes are distinguishable in the transcript, and
    /// Ran-and-found-nothing pushes nothing (silence) (spec §5 / §4).
    #[test]
    fn tui_distinguishes_the_three_outcomes() {
        let mut t = Transcript::new();
        apply_event(
            &mut t,
            AgentEvent::knowledge_supplied(KnowledgeOutcome::Off, Vec::new(), 0),
            false,
        );
        assert_eq!(last_block_text(&t), Some("memory off · recall disabled"));

        let mut t = Transcript::new();
        apply_event(
            &mut t,
            AgentEvent::knowledge_supplied(KnowledgeOutcome::Skipped, Vec::new(), 0),
            false,
        );
        assert_eq!(
            last_block_text(&t),
            Some("memory skipped · not permitted to read saved claims")
        );

        // Ran-and-found-nothing: nothing is pushed (silence).
        let mut t = Transcript::new();
        apply_event(
            &mut t,
            AgentEvent::knowledge_supplied(
                KnowledgeOutcome::Ran {
                    store_unavailable: false,
                },
                Vec::new(),
                0,
            ),
            false,
        );
        assert!(
            t.blocks().is_empty(),
            "Ran-nothing pushes nothing: {:?}",
            t.blocks()
        );
    }

    /// A non-zero dropped count is visible in the pushed block (spec §5).
    #[test]
    fn tui_shows_a_nonzero_dropped_count() {
        let mut t = Transcript::new();
        apply_event(
            &mut t,
            AgentEvent::knowledge_supplied(
                KnowledgeOutcome::Ran {
                    store_unavailable: false,
                },
                vec![dto_contract(vec![dto_claim(
                    "c-1",
                    "table_alias",
                    "orders",
                    ClaimStatus::Confirmed,
                )])],
                30,
            ),
            false,
        );
        let block = last_block_text(&t).expect("a block was pushed");
        assert!(block.contains("· 30 more dropped by bounds"), "{block}");
    }

    /// No opaque profile identity reaches the transcript: the profile name
    /// appears, a fabricated identity does not (spec §5 / §4).
    #[test]
    fn tui_does_not_leak_an_opaque_identity() {
        let fake_identity =
            "sha256:deadbeefcafef00d1234567890abcdef1234567890abcdef1234567890abcdef";
        let mut t = Transcript::new();
        apply_event(
            &mut t,
            AgentEvent::knowledge_supplied(
                KnowledgeOutcome::Ran {
                    store_unavailable: false,
                },
                vec![dto_contract(vec![dto_claim(
                    "c-1",
                    "table_alias",
                    "orders",
                    ClaimStatus::Confirmed,
                )])],
                0,
            ),
            false,
        );
        let block = last_block_text(&t).expect("a block was pushed");
        assert!(block.contains("analytics"), "profile name appears: {block}");
        assert!(!block.contains(fake_identity), "identity leaked: {block}");
    }

    /// A `TurnReset` (mid-stream failure, turn retrying) clears the assistant
    /// text accumulated so far, so the re-streamed answer **replaces** it —
    /// the transcript must never show the partial attempt concatenated with
    /// the full retry.
    #[test]
    fn turn_reset_replaces_the_accumulated_answer_rather_than_appending() {
        let mut t = Transcript::new();
        apply_event(&mut t, AgentEvent::assistant_text("The an"), false);
        apply_event(&mut t, AgentEvent::assistant_text("sw"), false);
        assert_eq!(last_block_text(&t), Some("The answ"));

        apply_event(&mut t, AgentEvent::turn_reset(), false);
        apply_event(
            &mut t,
            AgentEvent::assistant_text("The answer is 42."),
            false,
        );
        assert_eq!(
            last_block_text(&t),
            Some("The answer is 42."),
            "the retried answer must replace the partial text, not append to it"
        );
        // The re-streamed answer stays a single assistant block.
        assert_eq!(
            t.blocks()
                .iter()
                .filter(|b| b.kind == BlockKind::Assistant)
                .count(),
            1
        );
    }

    /// A reset with nothing streamed yet is a no-op: there is nothing to
    /// discard, and the next answer still opens its own block.
    #[test]
    fn turn_reset_with_nothing_streamed_is_a_no_op() {
        let mut t = Transcript::new();
        t.push(BlockKind::System, "memory off · recall disabled");
        apply_event(&mut t, AgentEvent::turn_reset(), false);
        assert_eq!(t.blocks().len(), 1, "nothing streamed → nothing to clear");
        apply_event(&mut t, AgentEvent::assistant_text("the answer"), false);
        assert_eq!(last_block_text(&t), Some("the answer"));
    }

    /// KnowledgeOverridden pushes a System block whose text names the referenced
    /// column and the specified value. Trails the answer — the
    /// block lands below the assistant text in the transcript.
    #[test]
    fn knowledge_overridden_pushes_a_system_block_naming_the_finding() {
        let mut t = Transcript::new();
        apply_event(
            &mut t,
            AgentEvent::knowledge_overridden(vec![OverrideFindingDto {
                claim_id: ClaimId::parse("c-rental-time").unwrap(),
                kind: "default_time_column".into(),
                claimed_value: "return_date".into(),
                observed_columns: vec!["rental_date".into()],
            }]),
            false,
        );
        let block = last_block_text(&t).expect("a block was pushed");
        assert!(
            block.contains("memory overridden · 1 finding"),
            "header: {block}"
        );
        assert!(
            block.contains("referenced rental_date"),
            "names the referenced column: {block}"
        );
        assert!(
            block.contains("where you specified return_date"),
            "names the specified value: {block}"
        );
        // The wording constraint: no causal "used" about the time column.
        assert!(
            !block.contains("used"),
            "the TUI block must not assert a causal 'used': {block}"
        );
        assert_eq!(t.blocks().last().unwrap().kind, BlockKind::System);
    }

    /// An empty finding set pushes nothing — silence.
    #[test]
    fn an_empty_knowledge_overridden_event_pushes_nothing() {
        let mut t = Transcript::new();
        apply_event(&mut t, AgentEvent::knowledge_overridden(Vec::new()), false);
        assert!(
            t.blocks().is_empty(),
            "no findings → no block: {:?}",
            t.blocks()
        );
    }

    /// KnowledgeLearningSkipped pushes a System block whose text names the skip
    /// reason (packet-54 decision 4 — the TUI renders it, it does not fall
    /// through to the catch-all that would drop it). Trails the answer.
    #[test]
    fn knowledge_learning_skipped_pushes_a_system_block_naming_the_reason() {
        let mut t = Transcript::new();
        apply_event(
            &mut t,
            AgentEvent::knowledge_learning_skipped(LearningSkipReason::TimedOut),
            false,
        );
        let block = last_block_text(&t).expect("a block was pushed");
        assert!(
            block.contains("memory not recorded · extraction timed out"),
            "TUI block names the timeout: {block}"
        );
        assert_eq!(t.blocks().last().unwrap().kind, BlockKind::System);

        let mut t = Transcript::new();
        apply_event(
            &mut t,
            AgentEvent::knowledge_learning_skipped(LearningSkipReason::Failed),
            false,
        );
        let block = last_block_text(&t).expect("a block was pushed");
        assert!(
            block.contains("memory not recorded · extraction failed"),
            "TUI block names the failure: {block}"
        );
    }

    /// The model's chain-of-thought reaches the transcript only when the user
    /// opted in. Off by default, so a user who did not ask for it never sees it;
    /// on, it lands as a `Thinking` block separate from the assistant answer.
    /// Asserts on the transcript state, the same seam the other `apply_event`
    /// tests use.
    #[test]
    fn reasoning_text_is_silent_when_thinking_is_off() {
        let mut t = Transcript::new();
        apply_event(
            &mut t,
            AgentEvent::reasoning_text("I considered the time column"),
            false,
        );
        assert!(
            t.blocks().is_empty(),
            "reasoning must not reach the transcript when thinking is off: {:?}",
            t.blocks()
        );
        // It stays silent even when an answer has already streamed — it does
        // not push a block above, below, or between assistant blocks.
        apply_event(&mut t, AgentEvent::assistant_text("the answer"), false);
        apply_event(
            &mut t,
            AgentEvent::reasoning_text("more thinking mid-turn"),
            false,
        );
        assert_eq!(
            t.blocks().len(),
            1,
            "only the assistant block should be present: {:?}",
            t.blocks()
        );
        assert_eq!(t.blocks()[0].kind, BlockKind::Assistant);
        assert!(
            !t.blocks()[0].text.contains("thinking"),
            "reasoning must not be folded into the assistant block: {:?}",
            t.blocks()[0].text
        );
    }

    /// When thinking is on, a `ReasoningText` event pushes a `Thinking` block
    /// carrying the chain-of-thought — separate from the assistant answer, so
    /// it cannot be mistaken for it. Empty reasoning pushes nothing either way.
    #[test]
    fn reasoning_text_pushes_a_thinking_block_when_thinking_is_on() {
        let mut t = Transcript::new();
        apply_event(
            &mut t,
            AgentEvent::reasoning_text("I considered the time column"),
            true,
        );
        assert_eq!(t.blocks().len(), 1, "one thinking block is pushed");
        assert_eq!(t.blocks()[0].kind, BlockKind::Thinking);
        assert_eq!(t.blocks()[0].text, "I considered the time column");

        // Empty reasoning pushes nothing, even with thinking on.
        apply_event(&mut t, AgentEvent::reasoning_text(""), true);
        assert_eq!(t.blocks().len(), 1, "empty reasoning pushes no block");
    }

    /// Displaying the chain-of-thought must not open a path for it to reach a
    /// persisted session. The reasoning restates row values and column
    /// contents in prose, and session files are redacted against a different
    /// threat, so the display toggle and the persistence boundary stay
    /// independent: turning the former on must not weaken the latter.
    ///
    /// What this pins is the display half: the reasoning really reaches the
    /// transcript as its own block. The session built alongside it is a
    /// separate object, so the absence checks below are a shape check on the
    /// persisted form, not proof that a live turn cannot carry reasoning into
    /// it — that guarantee is structural and pinned elsewhere, by
    /// `record_turn` taking no reasoning argument and by `ChatMessage`'s wire
    /// form being fixed by test.
    #[test]
    fn reasoning_shown_on_screen_stays_out_of_the_persisted_session() {
        use crate::interactive::session_state::SessionState;

        let reasoning = "the secret chain-of-thought about row values 9f3a";
        let mut transcript = Transcript::default();
        apply_event(&mut transcript, AgentEvent::reasoning_text(reasoning), true);
        apply_event(
            &mut transcript,
            AgentEvent::assistant_text("the answer is 42"),
            true,
        );

        assert!(
            transcript
                .blocks()
                .iter()
                .any(|b| b.kind == BlockKind::Thinking && b.text.contains(reasoning)),
            "the reasoning must be on screen for this test to prove anything"
        );

        let mut session =
            SessionState::new("s1", Some(String::from("analytics")), String::from("m"));
        session.show_thinking = true;
        session.record_turn("what is the answer", "the answer is 42", false, Vec::new());

        let json = serde_json::to_string(&session).expect("serializes");
        assert!(
            !json.contains(reasoning),
            "reasoning on screen leaked into the persisted session: {json}"
        );

        let replayed = session.provider_history();
        assert!(
            replayed.iter().all(|m| !m.content.contains(reasoning)),
            "reasoning on screen leaked into replayed history: {replayed:?}"
        );

        let redacted = session.redacted();
        let redacted_json = serde_json::to_string(&redacted).expect("serializes");
        assert!(
            !redacted_json.contains(reasoning),
            "reasoning on screen leaked into the redacted session: {redacted_json}"
        );
    }
}
