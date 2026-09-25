use crate::{RenderFormat, TerminalEvent, render::Rendered, render_event};
use async_trait::async_trait;
use saya_agent::{AgentEvent, AgentEventSink};
use std::{
    io::{self, Write},
    sync::Mutex,
};

pub(crate) struct TerminalSink {
    format: RenderFormat,
    text_open: Mutex<bool>,
    group: Mutex<TextGroupState>,
}
impl TerminalSink {
    pub(crate) fn new(format: RenderFormat) -> Self {
        Self {
            format,
            text_open: Mutex::new(false),
            group: Mutex::new(TextGroupState::new()),
        }
    }
}

#[async_trait]
impl AgentEventSink for TerminalSink {
    async fn emit(&self, event: AgentEvent) {
        let rendered = render_text_stream(
            event,
            self.format,
            &mut self.text_open.lock().expect("terminal state"),
            &mut self.group.lock().expect("terminal state"),
        );
        print!("{}", rendered.stdout);
        eprint!("{}", rendered.stderr);
        let _ = io::stdout().flush();
        let _ = io::stderr().flush();
    }
}

/// Append-only text groups collapse by buffer-then-decide: tool events
/// accumulate from group-open until the next boundary event, then the shaped
/// summary flushes where the group's first line would have printed. The
/// buffer is bounded — a force-flush every [`GROUP_CALL_BOUND`] completed
/// calls degrades a runaway group into chunk summaries, never unbounded
/// memory. Only the `Text` adapter groups; `Json`/`Ndjson` bypass entirely.
const GROUP_CALL_BOUND: usize = 32;

/// The pending run the text adapter has buffered but not yet decided on.
/// `completed` counts completed calls purely to bound the buffer (see
/// `GROUP_CALL_BOUND`); rendering decisions come from the shared grouper at
/// flush time, never from this counter.
#[derive(Debug, Default)]
struct TextGroupState {
    pending: Vec<AgentEvent>,
    completed: usize,
}

impl TextGroupState {
    fn new() -> Self {
        Self::default()
    }
}

fn is_group_member(event: &AgentEvent) -> bool {
    matches!(
        event,
        AgentEvent::ToolRequested { .. } | AgentEvent::ToolCompleted { .. }
    )
}

/// The stream entry point: `Text` groups tool runs through the shared
/// grouper before rendering; `Json`/`Ndjson` bypass the grouper entirely so
/// the machine surface stays event-for-event. The boundary rule mirrors the
/// grouper exactly — every non-member event flushes the open group, including
/// silent ones — so the pipe renders the shared grouping, never a second one.
fn render_text_stream(
    event: AgentEvent,
    format: RenderFormat,
    text_open: &mut bool,
    group: &mut TextGroupState,
) -> Rendered {
    if !matches!(format, RenderFormat::Text) {
        return render_agent(event, format, text_open);
    }
    if is_group_member(&event) {
        let completed_call = matches!(event, AgentEvent::ToolCompleted { .. });
        group.pending.push(event);
        if completed_call {
            group.completed += 1;
        }
        if group.completed >= GROUP_CALL_BOUND {
            return flush_group(group, text_open);
        }
        return Rendered {
            stdout: String::new(),
            stderr: String::new(),
        };
    }
    let mut out = flush_group(group, text_open);
    let rendered = render_agent(event, format, text_open);
    out.stdout.push_str(&rendered.stdout);
    out.stderr.push_str(&rendered.stderr);
    out
}

/// Shapes the buffered run into the Decision-2 summary (or today's verbatim
/// lines for a single call) and renders it through the text adapter,
/// preserving the assistant-delta close the ungrouped path applies.
fn flush_group(group: &mut TextGroupState, text_open: &mut bool) -> Rendered {
    if group.pending.is_empty() {
        return Rendered {
            stdout: String::new(),
            stderr: String::new(),
        };
    }
    let pending = std::mem::take(&mut group.pending);
    group.completed = 0;
    let mut out = Rendered {
        stdout: String::new(),
        stderr: String::new(),
    };
    let groups = crate::render::tool_groups::group_tool_events(&pending);
    for shaped in &groups {
        for line in crate::render::tool_groups::shape_group(shaped) {
            let line = crate::render::sanitize_terminal(&line);
            if *text_open {
                out.stdout.push('\n');
                *text_open = false;
            }
            out.stdout.push_str(&line);
            if !line.ends_with('\n') {
                out.stdout.push('\n');
            }
        }
    }
    out
}

fn render_agent(event: AgentEvent, format: RenderFormat, text_open: &mut bool) -> Rendered {
    let Some(event) = terminal_event(event) else {
        // Nothing to print, and nothing to close: a progress signal must not
        // terminate an open text block the way a real event would.
        return Rendered {
            stdout: String::new(),
            stderr: String::new(),
        };
    };
    let close = matches!(format, RenderFormat::Text)
        && *text_open
        && !matches!(event, TerminalEvent::AssistantText { .. });
    let mut rendered = render_event(&event, format);
    if close && !matches!(event, TerminalEvent::Complete) {
        rendered.stdout.insert(0, '\n');
        *text_open = false;
    }
    match event {
        TerminalEvent::AssistantText { ref text } if matches!(format, RenderFormat::Text) => {
            *text_open = !text.is_empty()
        }
        TerminalEvent::Complete if matches!(format, RenderFormat::Text) => {
            *text_open = false;
            if !close {
                rendered.stdout.clear();
            }
        }
        _ => {}
    }
    rendered
}

/// Maps one agent event to what the headless renderer should print, or `None`
/// when the event is a pure progress signal with nothing to say here.
///
/// The `None` case exists because the catch-all below is deliberately loud: a
/// variant this renderer does not understand must surface as
/// `NotImplemented` rather than silently end the stream. That is right for an
/// event carrying content, and wrong for one that carries none — a spinner
/// label is meaningful to the TUI and meaningless to a pipe. Without this
/// distinction, adding a progress event prints `unrecognized agent event` at
/// the user, which has now happened three times (see the arms below).
pub(crate) fn terminal_event(event: AgentEvent) -> Option<TerminalEvent> {
    Some(match event {
        AgentEvent::AssistantText { text } => TerminalEvent::AssistantText { text },
        AgentEvent::ToolRequested {
            name,
            arguments,
            effect,
        } => {
            let detail = crate::agent::tools::tool_call_detail(&name, &arguments);
            TerminalEvent::ToolRequested {
                name,
                detail,
                effect,
            }
        }
        AgentEvent::ToolCompleted { name, summary } => {
            TerminalEvent::ToolCompleted { name, summary }
        }
        AgentEvent::ToolDenied { name, reason } => TerminalEvent::ToolDenied { name, reason },
        AgentEvent::KnowledgeSupplied {
            outcome,
            contracts,
            dropped_by_bounds,
        } => TerminalEvent::KnowledgeSupplied {
            outcome,
            contracts,
            dropped_by_bounds,
        },
        AgentEvent::KnowledgeOverridden { findings } => {
            TerminalEvent::KnowledgeOverridden { findings }
        }
        // Extraction timed out or errored after the turn succeeded — surface it
        // rather than fall through to the `unrecognized agent event` catch-all.
        AgentEvent::KnowledgeLearningSkipped { reason } => {
            TerminalEvent::KnowledgeLearningSkipped { reason }
        }
        // The extraction circuit breaker tripped — surface it the same way,
        // rather than fall through to the `unrecognized agent event` catch-all.
        AgentEvent::KnowledgeLearningDisabled { model, misses } => {
            TerminalEvent::KnowledgeLearningDisabled { model, misses }
        }
        // Learning used to fall through to the catch-all below and print
        // `unrecognized agent event` — an error string at the exact moment the
        // product did the thing it is for.
        AgentEvent::KnowledgeProposed { claim } => TerminalEvent::KnowledgeLearned { claim },
        // Progress only: the TUI labels its spinner with this, and a pipe has
        // no spinner to label. Dropped rather than rendered — not forgotten,
        // which is what the catch-all would make of it.
        AgentEvent::KnowledgeLearningStarted => return None,
        // A provider attempt began. The TUI records a rollback watermark here;
        // a pipe has nothing to roll back and nothing to say. Explicitly None
        // rather than left to the catch-all, which would print `unrecognized
        // agent event` at the user — the regression the note above says has
        // already shipped three times for contentless variants.
        AgentEvent::TurnStarted => return None,
        // The model's chain-of-thought. This is the one case where rendering to
        // nothing is a *scope* decision rather than a *nature-of-the-event*
        // decision: reasoning is content (it mirrors `AssistantText`), so by its
        // nature it would belong on the loud path below — but display belongs
        // to the interactive transcript, not a pipe, and nothing is displayed
        // by default. So it renders to `None` here, the same way a progress
        // signal does, for a different reason. The tests pin both halves: this
        // arm stays silent, and a content event (`AssistantText`, `Complete`)
        // still reaches the loud path — so a future reader cannot conclude
        // reasoning is progress, and a future change cannot silence the
        // catch-all to pass one and break the other.
        AgentEvent::ReasoningText { .. } => return None,
        // The token counts one provider call reported. This is data the JSON/NDJSON
        // adapter carries whole; the text adapter renders it to nothing (the
        // interactive surfaces for it are the per-turn token line and `/usage`,
        // and a pipe's reader has the answer above it). It must not fall through
        // to the catch-all below, which would print `unrecognized agent event`
        // under a correct answer — the same regression that has shipped for
        // contentless variants here three times.
        AgentEvent::Usage { call, usage } => TerminalEvent::Usage { call, usage },
        AgentEvent::AnswerDesignated { sql } => TerminalEvent::AnswerDesignated { sql },
        AgentEvent::ConsensusDecided {
            sql,
            attempts,
            voted,
            votes,
            margin,
            tied,
            probe_broke_tie,
        } => TerminalEvent::ConsensusDecided {
            sql,
            attempts,
            voted,
            votes,
            margin,
            tied,
            probe_broke_tie,
        },
        AgentEvent::Complete => TerminalEvent::Complete,
        // The turn's provider stream failed mid-answer and the loop is
        // retrying it. Carried under its own type tag so a machine consumer
        // can replace the text it accumulated for this turn; the text adapter
        // prints a notice (see `TerminalEvent::TurnReset`).
        AgentEvent::TurnReset => TerminalEvent::TurnReset,
        // AgentEvent is #[non_exhaustive]; a future variant this renderer does not
        // yet understand must not silently terminate the stream (Complete) — surface
        // it as an unimplemented event instead.
        _ => TerminalEvent::NotImplemented {
            feature: "unrecognized agent event".into(),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::{RenderFormat, render_event};
    use saya_agent::{
        KnowledgeOutcome, LearningSkipReason, LocalStateEffect, OverrideFindingDto,
        SuppliedClaimDto, SuppliedContractDto, ToolEffect,
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

    /// KnowledgeSupplied maps to a real TerminalEvent variant (not
    /// NotImplemented) and renders through the text adapter.
    #[test]
    fn knowledge_supplied_renders_through_the_text_adapter() {
        let event = AgentEvent::knowledge_supplied(
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
        );
        let rendered = render_agent(event, RenderFormat::Text, &mut false);
        // The compact header, the unconfirmed count (pointing at /queue),
        // and the per-claim lines all reach stdout through the adapter.
        assert!(
            rendered
                .stdout
                .contains("memory supplied · 2 claims (1 unconfirmed — review with /queue)"),
            "{:?}",
            rendered.stdout
        );
        assert!(
            rendered.stdout.contains("table_alias  orders  confirmed"),
            "{:?}",
            rendered.stdout
        );
        assert!(
            rendered.stdout.contains("candidate  (unconfirmed)"),
            "{:?}",
            rendered.stdout
        );
        assert_eq!(rendered.stderr, "");
    }

    /// The three outcomes render distinguishably through the text adapter, and
    /// Ran-and-found-nothing is silent (spec §5 / §4).
    #[test]
    fn text_adapter_distinguishes_the_three_outcomes() {
        let off = render_agent(
            AgentEvent::knowledge_supplied(KnowledgeOutcome::Off, Vec::new(), 0),
            RenderFormat::Text,
            &mut false,
        );
        let skipped = render_agent(
            AgentEvent::knowledge_supplied(KnowledgeOutcome::Skipped, Vec::new(), 0),
            RenderFormat::Text,
            &mut false,
        );
        let ran_empty = render_agent(
            AgentEvent::knowledge_supplied(
                KnowledgeOutcome::Ran {
                    store_unavailable: false,
                },
                Vec::new(),
                0,
            ),
            RenderFormat::Text,
            &mut false,
        );
        assert_eq!(off.stdout, "memory off · recall disabled\n");
        assert_eq!(
            skipped.stdout,
            "memory skipped · not permitted to read saved claims\n"
        );
        assert_eq!(ran_empty.stdout, "", "Ran-and-found-nothing is silent");
        assert_ne!(off.stdout, skipped.stdout);
    }

    /// A non-zero dropped count is visible through the text adapter (spec §5).
    #[test]
    fn text_adapter_shows_a_nonzero_dropped_count() {
        let rendered = render_agent(
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
            RenderFormat::Text,
            &mut false,
        );
        assert!(
            rendered.stdout.contains("· 30 more dropped by bounds"),
            "{:?}",
            rendered.stdout
        );
    }

    /// The JSON/NDJSON adapter carries the event under its type tag rather than
    /// the NotImplemented fallback (spec §2: every adapter that renders events).
    #[test]
    fn json_adapter_carries_the_event_under_its_type_tag() {
        let event = AgentEvent::knowledge_supplied(
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
        );
        let te = terminal_event(event).expect("this event renders headlessly");
        let rendered = render_event(&te, RenderFormat::Json);
        assert!(
            rendered.stdout.contains(r#""event":"knowledge_supplied""#),
            "{:?}",
            rendered.stdout
        );
        // The NotImplemented fallback is not what the JSON adapter emits here.
        assert!(
            !rendered.stdout.contains("not_implemented"),
            "{:?}",
            rendered.stdout
        );
    }

    #[test]
    fn text_status_closes_an_open_delta_line_once() {
        let mut open = false;
        assert_eq!(
            render_agent(
                AgentEvent::assistant_text("thinking"),
                RenderFormat::Text,
                &mut open
            )
            .stdout,
            "thinking"
        );
        // `schema_discovery`'s declared effect: touches nothing and reaches
        // nothing, so the read-only claim is earned and the line keeps it.
        assert_eq!(
            render_agent(
                AgentEvent::tool_requested(
                    "schema",
                    serde_json::Value::Null,
                    Some(ToolEffect {
                        database_data: false,
                        external_side_effect: false,
                        requires_approval: false,
                        local_state: LocalStateEffect::None,
                    }),
                ),
                RenderFormat::Text,
                &mut open
            )
            .stdout,
            "\nUsing read-only tool: schema\n"
        );
        assert_eq!(
            render_agent(AgentEvent::complete(), RenderFormat::Text, &mut open).stdout,
            ""
        );
    }

    /// The write-shaped class — `workspace_write` and `scratch_sql` both
    /// declare `LocalStateEffect::WriteWorkspace` — must never be announced as
    /// read-only. The old renderer hardcoded the claim on the request line, so
    /// a run that wrote a file announced "Using read-only tool: workspace_write"
    /// at the exact moment it was about to write. The label is derived from the
    /// declaration the loop carries on the event; against the old code this
    /// test fails, because the claim was hardcoded regardless of the effect.
    #[test]
    fn a_write_shaped_tool_is_never_announced_as_read_only() {
        let rendered = render_agent(
            AgentEvent::tool_requested(
                "workspace_write",
                serde_json::json!({"path": "notes.md", "content": "the run's note"}),
                Some(ToolEffect {
                    database_data: false,
                    external_side_effect: false,
                    requires_approval: false,
                    local_state: LocalStateEffect::WriteWorkspace,
                }),
            ),
            RenderFormat::Text,
            &mut false,
        );
        assert_eq!(
            rendered.stdout, "Using tool: workspace_write\n  notes.md\n",
            "a write-shaped tool gets the claim-free line with the named file: {:?}",
            rendered.stdout
        );
        assert!(
            !rendered.stdout.contains("read-only"),
            "the read-only claim must not appear for a write-shaped tool: {:?}",
            rendered.stdout
        );
        assert_eq!(rendered.stderr, "");
    }

    /// A side-effecting tool — one whose declaration carries
    /// `external_side_effect`, like `http_fetch` and `http_download` — must
    /// never be announced as read-only either, with or without a visible call
    /// detail (the detail arm is `render_chart`'s: SQL shown, effect still
    /// side-effecting). The old code printed "Using read-only tool" for every
    /// tool, so both assertions failed against it.
    #[test]
    fn a_side_effecting_tool_is_never_announced_as_read_only() {
        let effect = ToolEffect {
            database_data: false,
            external_side_effect: true,
            requires_approval: false,
            local_state: LocalStateEffect::None,
        };
        let fetched = render_agent(
            AgentEvent::tool_requested(
                "http_fetch",
                serde_json::json!({"url": "https://example.com/feed"}),
                Some(effect),
            ),
            RenderFormat::Text,
            &mut false,
        );
        assert_eq!(
            fetched.stdout, "Using tool: http_fetch\n",
            "a side-effecting tool gets the claim-free line: {:?}",
            fetched.stdout
        );
        assert!(
            !fetched.stdout.contains("read-only"),
            "the read-only claim must not appear for a side-effecting tool: {:?}",
            fetched.stdout
        );

        // The detail arm: a side-effecting tool with a visible call detail
        // keeps the detail line and still carries no read-only claim.
        let charted = render_agent(
            AgentEvent::tool_requested(
                "render_chart",
                serde_json::json!({"sql": "SELECT 1", "chart_type": "bar"}),
                Some(effect),
            ),
            RenderFormat::Text,
            &mut false,
        );
        assert_eq!(
            charted.stdout, "Using tool: render_chart\n  SELECT 1  (chart: bar)\n",
            "the detail survives on the claim-free line: {:?}",
            charted.stdout
        );
        assert!(
            !charted.stdout.contains("read-only"),
            "the read-only claim must not appear beside a detail either: {:?}",
            charted.stdout
        );
    }
    #[test]
    fn ndjson_keeps_delta_and_complete_envelopes() {
        let mut open = false;
        assert_eq!(
            render_agent(
                AgentEvent::assistant_text("x"),
                RenderFormat::Ndjson,
                &mut open
            )
            .stdout,
            "{\"event\":\"assistant_text\",\"text\":\"x\"}\n"
        );
        assert_eq!(
            render_agent(AgentEvent::complete(), RenderFormat::Ndjson, &mut open).stdout,
            "{\"event\":\"complete\"}\n"
        );
    }
    #[test]
    fn text_complete_closes_an_open_delta_line_once() {
        let mut open = false;
        let _ = render_agent(
            AgentEvent::assistant_text("done"),
            RenderFormat::Text,
            &mut open,
        );
        assert_eq!(
            render_agent(AgentEvent::complete(), RenderFormat::Text, &mut open).stdout,
            "\n"
        );
    }

    /// KnowledgeOverridden maps to a real TerminalEvent variant (not
    /// NotImplemented) and renders through the text adapter.
    #[test]
    fn knowledge_overridden_renders_through_the_text_adapter() {
        let event = AgentEvent::knowledge_overridden(vec![OverrideFindingDto {
            claim_id: ClaimId::parse("c-rental-time").unwrap(),
            kind: "default_time_column".into(),
            claimed_value: "return_date".into(),
            observed_columns: vec!["rental_date".into()],
        }]);
        let rendered = render_agent(event, RenderFormat::Text, &mut false);
        assert!(
            rendered.stdout.contains("memory overridden · 1 finding"),
            "{:?}",
            rendered.stdout
        );
        assert!(
            rendered.stdout.contains("referenced rental_date"),
            "{:?}",
            rendered.stdout
        );
        // The wording constraint: no causal "used" about the time column.
        assert!(
            !rendered.stdout.contains("used"),
            "the text adapter must not assert a causal 'used': {:?}",
            rendered.stdout
        );
        assert_eq!(rendered.stderr, "");
    }

    /// The JSON/NDJSON adapter carries KnowledgeOverridden under its type tag
    /// rather than the NotImplemented fallback (spec §2: every adapter).
    #[test]
    fn json_adapter_carries_the_overridden_event_under_its_type_tag() {
        let event = AgentEvent::knowledge_overridden(vec![OverrideFindingDto {
            claim_id: ClaimId::parse("c-rental-time").unwrap(),
            kind: "default_time_column".into(),
            claimed_value: "return_date".into(),
            observed_columns: vec!["rental_date".into()],
        }]);
        let te = terminal_event(event).expect("this event renders headlessly");
        let rendered = render_event(&te, RenderFormat::Json);
        assert!(
            rendered
                .stdout
                .contains(r#""event":"knowledge_overridden""#),
            "{:?}",
            rendered.stdout
        );
        assert!(
            !rendered.stdout.contains("not_implemented"),
            "{:?}",
            rendered.stdout
        );
    }

    /// KnowledgeLearningSkipped maps to a real TerminalEvent variant (not
    /// NotImplemented) and renders the spec line through the text adapter
    /// (packet-54 decision 4 — both adapters render it).
    #[test]
    fn knowledge_learning_skipped_renders_through_the_text_adapter() {
        let event = AgentEvent::knowledge_learning_skipped(LearningSkipReason::TimedOut);
        let rendered = render_agent(event, RenderFormat::Text, &mut false);
        assert!(
            rendered
                .stdout
                .contains("memory not recorded · extraction timed out"),
            "text adapter renders the timeout line: {:?}",
            rendered.stdout
        );
        assert_eq!(rendered.stderr, "");

        let event = AgentEvent::knowledge_learning_skipped(LearningSkipReason::Failed);
        let rendered = render_agent(event, RenderFormat::Text, &mut false);
        assert!(
            rendered
                .stdout
                .contains("memory not recorded · extraction failed"),
            "text adapter renders the failure line: {:?}",
            rendered.stdout
        );
    }

    /// The JSON/NDJSON adapter carries KnowledgeLearningSkipped under its type
    /// tag rather than the NotImplemented fallback (packet-54 decision 4: every
    /// adapter that renders events — the headless path previously would have
    /// printed `unrecognized agent event`).
    #[test]
    fn json_adapter_carries_the_learning_skipped_event_under_its_type_tag() {
        let event = AgentEvent::knowledge_learning_skipped(LearningSkipReason::TimedOut);
        let te = terminal_event(event).expect("this event renders headlessly");
        let rendered = render_event(&te, RenderFormat::Json);
        assert!(
            rendered
                .stdout
                .contains(r#""event":"knowledge_learning_skipped""#),
            "type tag: {:?}",
            rendered.stdout
        );
        assert!(
            rendered.stdout.contains(r#""reason":"timed_out""#),
            "reason carried: {:?}",
            rendered.stdout
        );
        assert!(
            !rendered.stdout.contains("not_implemented"),
            "must not fall through to NotImplemented: {:?}",
            rendered.stdout
        );
    }

    /// KnowledgeLearningDisabled maps to a real TerminalEvent variant (not
    /// NotImplemented) and renders the spec line through the text adapter.
    #[test]
    fn knowledge_learning_disabled_renders_through_the_text_adapter() {
        let event = AgentEvent::knowledge_learning_disabled("glm-5.2", 2);
        let rendered = render_agent(event, RenderFormat::Text, &mut false);
        assert!(
            rendered
                .stdout
                .contains("memory: learning disabled for this session"),
            "text adapter renders the disabled line: {:?}",
            rendered.stdout
        );
        assert!(
            rendered.stdout.contains("glm-5.2") && rendered.stdout.contains('2'),
            "the model and miss count appear: {:?}",
            rendered.stdout
        );
        assert_eq!(rendered.stderr, "");
    }

    /// The JSON/NDJSON adapter carries KnowledgeLearningDisabled under its
    /// type tag rather than the NotImplemented fallback, with the model and
    /// miss count on the wire.
    #[test]
    fn json_adapter_carries_the_learning_disabled_event_under_its_type_tag() {
        let event = AgentEvent::knowledge_learning_disabled("glm-5.2", 2);
        let te = terminal_event(event).expect("this event renders headlessly");
        let rendered = render_event(&te, RenderFormat::Json);
        assert!(
            rendered
                .stdout
                .contains(r#""event":"knowledge_learning_disabled""#),
            "type tag: {:?}",
            rendered.stdout
        );
        assert!(
            rendered.stdout.contains(r#""model":"glm-5.2""#),
            "model carried: {:?}",
            rendered.stdout
        );
        assert!(
            rendered.stdout.contains(r#""misses":2"#),
            "misses carried: {:?}",
            rendered.stdout
        );
        assert!(
            !rendered.stdout.contains("not_implemented"),
            "{:?}",
            rendered.stdout
        );
    }

    /// A progress-only event must render to nothing, not to
    /// `unrecognized agent event`. The catch-all is deliberately loud so a
    /// content-bearing variant cannot be dropped silently, which means every
    /// contentless one needs an explicit arm — this has been missed three times
    /// (`KnowledgeLearningSkipped`, `KnowledgeProposed`, and
    /// `KnowledgeLearningStarted`, which printed the error string during a live
    /// `saya ask` while the whole suite was green).
    #[test]
    fn progress_only_events_render_to_nothing_not_to_an_error() {
        assert!(
            terminal_event(AgentEvent::KnowledgeLearningStarted).is_none(),
            "a progress signal must not reach the headless renderer"
        );

        // And the loud path still works for a variant that does carry content.
        let complete = terminal_event(AgentEvent::Complete).expect("Complete renders");
        assert!(
            !matches!(complete, TerminalEvent::NotImplemented { .. }),
            "a known content event must not fall through to the catch-all"
        );
    }

    /// `ReasoningText` carries content (chain-of-thought), so by its nature it
    /// would reach the loud catch-all and print
    /// `Not implemented: unrecognized agent event` under a correct answer in the
    /// headless `saya ask` path — exactly the regression that shipped green
    /// three times. The headless renderer displays nothing for reasoning: it is
    /// a *scope* decision (display belongs to the interactive transcript, not a
    /// pipe) rather than a *nature-of-the-event* decision (reasoning is content,
    /// not progress). The assertion pins the scope choice so a future reader
    /// cannot conclude reasoning is progress.
    #[test]
    fn reasoning_text_renders_to_nothing_in_the_headless_path() {
        assert!(
            terminal_event(AgentEvent::reasoning_text("I considered the time column")).is_none(),
            "reasoning must not reach the headless renderer, and must not fall \
             through to the `unrecognized agent event` catch-all"
        );
    }

    /// The fix is not a blanket silence. `ReasoningText` renders to `None`, but
    /// a content event the headless renderer *does* understand still reaches a
    /// real `TerminalEvent` and never the `NotImplemented` catch-all. Without
    /// this, silencing reasoning by widening the catch-all would pass the test
    /// above and quietly break every other variant.
    #[test]
    fn silencing_reasoning_does_not_silence_a_content_event() {
        // `AssistantText` is the variant `ReasoningText` mirrors — content, and
        // the headless renderer must still surface it.
        let text = terminal_event(AgentEvent::assistant_text("the answer")).expect("renders");
        assert!(
            matches!(text, TerminalEvent::AssistantText { .. }),
            "a content event must reach a real TerminalEvent, not be silenced: {text:?}"
        );
        // And `Complete` — the other content-bearing terminator — still renders.
        let complete = terminal_event(AgentEvent::complete()).expect("renders");
        assert!(
            !matches!(complete, TerminalEvent::NotImplemented { .. }),
            "Complete must not fall through to the catch-all: {complete:?}"
        );
    }

    /// The designated answering SQL reaches the NDJSON stream under its own type
    /// tag so a harness can pair the prose answer with its query, and the text
    /// adapter stays silent (the SQL was already shown when the query ran).
    #[test]
    fn answer_designated_reaches_ndjson_and_stays_silent_in_text() {
        let event = AgentEvent::answer_designated("SELECT count(*) FROM t");
        let terminal = terminal_event(event).expect("designation renders headlessly");
        let json = render_event(&terminal, RenderFormat::Ndjson);
        assert!(
            json.stdout.contains(r#""event":"answer_designated""#),
            "ndjson must tag the designation: {json:?}"
        );
        assert!(
            json.stdout.contains("SELECT count(*) FROM t"),
            "ndjson must carry the SQL: {json:?}"
        );
        let text = render_event(&terminal, RenderFormat::Text);
        assert!(
            text.stdout.is_empty(),
            "the text adapter must not echo the designation: {text:?}"
        );
    }

    /// The consensus decision reaches the NDJSON stream under its own type tag
    /// (so a harness can read the vote tallies) and prints a text line naming
    /// the believed query — never falling through to the `unrecognized agent
    /// event` catch-all. Carries the SQL only; no result rows.
    #[test]
    fn consensus_decided_reaches_ndjson_and_names_the_winner_in_text() {
        let event = AgentEvent::consensus_decided(
            Some("SELECT count(*) FROM t".into()),
            3,
            3,
            3,
            3,
            false,
            false,
        );
        let terminal = terminal_event(event).expect("consensus renders headlessly");
        assert!(
            !matches!(terminal, TerminalEvent::NotImplemented { .. }),
            "ConsensusDecided must not fall through to the catch-all: {terminal:?}"
        );
        let json = render_event(&terminal, RenderFormat::Ndjson);
        assert!(
            json.stdout.contains(r#""event":"consensus_decided""#),
            "ndjson must tag the consensus: {json:?}"
        );
        assert!(
            json.stdout.contains("SELECT count(*) FROM t"),
            "ndjson must carry the winning SQL, not result rows: {json:?}"
        );
        assert!(
            json.stdout.contains(r#""attempts":3"#) && json.stdout.contains(r#""votes":3"#),
            "ndjson must carry the tallies: {json:?}"
        );
        let text = render_event(&terminal, RenderFormat::Text);
        assert!(
            text.stdout
                .contains("consensus · 3 attempts, 3 voted, 3 agreed"),
            "the text adapter names the tallies: {text:?}"
        );
        assert!(
            text.stdout.contains("believing: SELECT count(*) FROM t"),
            "the text adapter names the believed query: {text:?}"
        );

        // A tie with no winner names the disagreement honestly, not as a guess.
        let tied = AgentEvent::consensus_decided(None, 3, 3, 1, 0, true, false);
        let tied_terminal = terminal_event(tied).expect("renders");
        let tied_text = render_event(&tied_terminal, RenderFormat::Text);
        assert!(
            tied_text.stdout.contains("tied, no winner"),
            "a tie with no winner says so: {tied_text:?}"
        );
    }

    /// The token counts one provider call reported reach the NDJSON stream
    /// under their own type tag, named by which call they describe so a
    /// consumer can keep the answer's cost apart from the extraction call's.
    #[test]
    fn usage_reaches_ndjson_under_its_type_tag_named_by_call() {
        let event = AgentEvent::usage(
            saya_agent::UsageCall::Extraction,
            saya_agent::TokenUsage::new(40, 10),
        );
        let terminal = terminal_event(event).expect("usage renders headlessly");
        assert!(
            !matches!(terminal, TerminalEvent::NotImplemented { .. }),
            "Usage must not fall through to the catch-all: {terminal:?}"
        );
        let json = render_event(&terminal, RenderFormat::Ndjson);
        assert!(
            json.stdout.contains(r#""event":"usage""#),
            "ndjson must tag the usage event: {json:?}"
        );
        assert!(
            json.stdout.contains(r#""call":"extraction""#),
            "the call kind must name the extraction call: {json:?}"
        );
        assert!(
            json.stdout.contains(r#""input_tokens":40"#)
                && json.stdout.contains(r#""output_tokens":10"#),
            "the counts must be carried: {json:?}"
        );

        let answering = render_event(
            &terminal_event(AgentEvent::usage(
                saya_agent::UsageCall::Answer,
                saya_agent::TokenUsage::new(3, 7),
            ))
            .expect("renders"),
            RenderFormat::Ndjson,
        );
        assert!(
            answering.stdout.contains(r#""call":"answer""#),
            "the answering call is named too: {answering:?}"
        );
    }

    /// A reported zero and an unreported number must not collapse on the wire.
    /// `cached_input_tokens` is `Option` precisely so "the provider says zero
    /// cached tokens" stays a claim it made, and "the provider said nothing"
    /// stays `null` — a cache hit rate computed over the two has to be able to
    /// tell "0%" from "unknown".
    #[test]
    fn reported_zero_and_unreported_cached_tokens_serialize_distinctly() {
        let reported_zero = render_event(
            &terminal_event(AgentEvent::usage(
                saya_agent::UsageCall::Answer,
                saya_agent::TokenUsage::new(10, 5).with_cached_input(Some(0)),
            ))
            .expect("renders"),
            RenderFormat::Ndjson,
        );
        assert!(
            reported_zero.stdout.contains(r#""cached_input_tokens":0"#),
            "a reported zero must serialize as a number: {:?}",
            reported_zero.stdout
        );
        let unreported = render_event(
            &terminal_event(AgentEvent::usage(
                saya_agent::UsageCall::Answer,
                saya_agent::TokenUsage::new(10, 5),
            ))
            .expect("renders"),
            RenderFormat::Ndjson,
        );
        assert!(
            unreported.stdout.contains(r#""cached_input_tokens":null"#),
            "an unreported figure must serialize as null: {:?}",
            unreported.stdout
        );
        assert_ne!(
            reported_zero.stdout, unreported.stdout,
            "the two must be distinguishable on the wire"
        );
    }

    /// The text adapter renders nothing for a usage event: the interactive
    /// surfaces for it already exist (the per-turn token line and `/usage`),
    /// and it must not fall through to `Not implemented: unrecognized agent
    /// event` under a correct answer.
    #[test]
    fn usage_renders_to_nothing_in_the_text_adapter() {
        let event = AgentEvent::usage(
            saya_agent::UsageCall::Answer,
            saya_agent::TokenUsage::new(3, 7),
        );
        let rendered = render_agent(event, RenderFormat::Text, &mut false);
        assert_eq!(rendered.stdout, "", "the text adapter stays silent");
        assert_eq!(rendered.stderr, "", "nothing on stderr either");
    }

    /// A usage event carries no content, so a text consumer that ignores it
    /// sees byte-identical output whether or not it arrives. It renders to
    /// nothing itself, and it closes an open delta line the way any other
    /// non-assistant event does — it arrives after the answer's text finished
    /// streaming, where closing is what `ToolRequested` already does. The
    /// property pinned is the invariant on the stream's shape: interleaving
    /// usage into a run changes nothing a text reader sees.
    #[test]
    fn interleaving_usage_leaves_the_text_output_unchanged() {
        let mut open = false;
        let answer = render_agent(
            AgentEvent::assistant_text("thinking"),
            RenderFormat::Text,
            &mut open,
        );
        let closed = render_agent(AgentEvent::complete(), RenderFormat::Text, &mut open);
        let without_usage = format!("{}{}", answer.stdout, closed.stdout);

        let mut open = false;
        let answer = render_agent(
            AgentEvent::assistant_text("thinking"),
            RenderFormat::Text,
            &mut open,
        );
        let usage = render_agent(
            AgentEvent::usage(
                saya_agent::UsageCall::Answer,
                saya_agent::TokenUsage::new(3, 7),
            ),
            RenderFormat::Text,
            &mut open,
        );
        let closed = render_agent(AgentEvent::complete(), RenderFormat::Text, &mut open);
        let with_usage = format!("{}{}{}", answer.stdout, usage.stdout, closed.stdout);

        assert_eq!(
            with_usage, without_usage,
            "usage must not change the text stream"
        );
        // And the delta line was still closed, not left hanging open.
        assert!(!open, "the line must be closed after Complete");
    }

    /// The JSON adapter carries the usage event under its type tag rather than
    /// the NotImplemented fallback (every adapter that renders events).
    #[test]
    fn json_adapter_carries_the_usage_event_under_its_type_tag() {
        let event = AgentEvent::usage(
            saya_agent::UsageCall::Answer,
            saya_agent::TokenUsage::new(3, 7).with_cached_input(Some(2)),
        );
        let te = terminal_event(event).expect("this event renders headlessly");
        let rendered = render_event(&te, RenderFormat::Json);
        assert!(
            rendered.stdout.contains(r#""event":"usage""#),
            "{:?}",
            rendered.stdout
        );
        assert!(
            !rendered.stdout.contains("not_implemented"),
            "{:?}",
            rendered.stdout
        );
    }

    /// `TurnReset` maps to a real TerminalEvent variant (not the
    /// NotImplemented catch-all) — a retry signal the renderer understands
    /// must never print `unrecognized agent event`.
    #[test]
    fn turn_reset_renders_as_its_own_variant_not_the_catch_all() {
        let terminal = terminal_event(AgentEvent::turn_reset()).expect("turn reset renders");
        assert!(
            matches!(terminal, TerminalEvent::TurnReset),
            "must map to the dedicated variant: {terminal:?}"
        );
    }

    /// The text adapter closes the open delta line and prints a notice, so a
    /// restart mid-answer is legible rather than reading as the model
    /// repeating itself; the retried deltas then start on a fresh line.
    #[test]
    fn turn_reset_closes_the_partial_answer_and_notes_the_retry_in_text() {
        let mut open = false;
        let _ = render_agent(
            AgentEvent::assistant_text("The an"),
            RenderFormat::Text,
            &mut open,
        );
        assert!(open, "the partial answer leaves the delta line open");

        let reset = render_agent(AgentEvent::turn_reset(), RenderFormat::Text, &mut open);
        assert_eq!(
            reset.stdout, "\nprovider stream interrupted — retrying\n",
            "the open line closes and the notice prints on its own line"
        );
        assert!(!open, "the reset closes the delta line");

        // The re-streamed answer starts a fresh line, not appended to the
        // partial text the reset discarded.
        let retried = render_agent(
            AgentEvent::assistant_text("The answer is 42."),
            RenderFormat::Text,
            &mut open,
        );
        assert_eq!(retried.stdout, "The answer is 42.");
        assert!(open);
    }

    /// The JSON/NDJSON adapter carries the reset under its type tag so a
    /// machine consumer can replace accumulated text instead of appending.
    #[test]
    fn turn_reset_reaches_ndjson_under_its_type_tag() {
        let terminal = terminal_event(AgentEvent::turn_reset()).expect("renders");
        let json = render_event(&terminal, RenderFormat::Ndjson);
        assert!(
            json.stdout.contains(r#""event":"turn_reset""#),
            "ndjson must tag the reset: {json:?}"
        );
        assert!(
            !json.stdout.contains("not_implemented"),
            "must not fall through to NotImplemented: {json:?}"
        );
    }

    fn drain_text_stream(events: Vec<AgentEvent>) -> String {
        let mut open = false;
        let mut group = TextGroupState::new();
        let mut stdout = String::new();
        for event in events {
            let rendered = render_text_stream(event, RenderFormat::Text, &mut open, &mut group);
            stdout.push_str(&rendered.stdout);
        }
        stdout.push_str(&flush_group(&mut group, &mut open).stdout);
        stdout
    }

    fn drain_ndjson_stream(events: Vec<AgentEvent>) -> String {
        let mut open = false;
        let mut group = TextGroupState::new();
        let mut stdout = String::new();
        for event in events {
            let rendered = render_text_stream(event, RenderFormat::Ndjson, &mut open, &mut group);
            stdout.push_str(&rendered.stdout);
        }
        stdout.push_str(&flush_group(&mut group, &mut open).stdout);
        stdout
    }

    fn write_effect() -> ToolEffect {
        ToolEffect {
            database_data: false,
            external_side_effect: false,
            requires_approval: false,
            local_state: LocalStateEffect::WriteWorkspace,
        }
    }

    fn run_effect() -> ToolEffect {
        ToolEffect {
            database_data: false,
            external_side_effect: true,
            requires_approval: false,
            local_state: LocalStateEffect::WriteWorkspace,
        }
    }

    fn mixed_sequence() -> Vec<AgentEvent> {
        vec![
            AgentEvent::assistant_text("the plan"),
            AgentEvent::tool_requested(
                "workspace_write",
                serde_json::json!({"path": "notes.md", "content": "hi"}),
                Some(write_effect()),
            ),
            AgentEvent::ToolCompleted {
                name: "workspace_write".into(),
                summary: "notes.md written".into(),
            },
            AgentEvent::tool_requested(
                "run_command",
                serde_json::json!({"program": "pytest", "args": ["-q"]}),
                Some(run_effect()),
            ),
            AgentEvent::ToolCompleted {
                name: "run_command".into(),
                summary: "failed pytest".into(),
            },
            AgentEvent::assistant_text("done"),
            AgentEvent::complete(),
        ]
    }

    /// C2 property 1: a run of successful calls through the piped text
    /// surface emits one summary line, not a line per call.
    #[test]
    fn piped_text_run_of_successful_calls_emits_one_summary_line() {
        let write = ToolEffect {
            database_data: false,
            external_side_effect: false,
            requires_approval: false,
            local_state: LocalStateEffect::WriteWorkspace,
        };
        let events = vec![
            AgentEvent::tool_requested(
                "workspace_write",
                serde_json::json!({"path": "notes.md", "content": "hi"}),
                Some(write),
            ),
            AgentEvent::ToolCompleted {
                name: "workspace_write".into(),
                summary: "notes.md written".into(),
            },
            AgentEvent::tool_requested(
                "workspace_write",
                serde_json::json!({"path": "other.md", "content": "hi"}),
                Some(write),
            ),
            AgentEvent::ToolCompleted {
                name: "workspace_write".into(),
                summary: "other.md written".into(),
            },
        ];
        let stdout = drain_text_stream(events);
        assert_eq!(
            stdout.lines().count(),
            1,
            "a run of successful calls is one summary line, not a line per call: {stdout:?}"
        );
        assert!(
            stdout.contains("2 tool calls · ok"),
            "the summary line collapses the run: {stdout:?}"
        );
    }

    /// C2 property 2: a group with a failure emits the header plus that
    /// failure's full pair — today's request and completion lines verbatim.
    #[test]
    fn piped_text_group_with_failure_emits_header_plus_full_pair() {
        let events = vec![
            AgentEvent::tool_requested(
                "workspace_write",
                serde_json::json!({"path": "notes.md", "content": "hi"}),
                Some(write_effect()),
            ),
            AgentEvent::ToolCompleted {
                name: "workspace_write".into(),
                summary: "notes.md written".into(),
            },
            AgentEvent::tool_requested(
                "run_command",
                serde_json::json!({"program": "pytest", "args": ["-q"]}),
                Some(run_effect()),
            ),
            AgentEvent::ToolCompleted {
                name: "run_command".into(),
                summary: "failed pytest".into(),
            },
            AgentEvent::complete(),
        ];
        let stdout = drain_text_stream(events);
        let lines: Vec<&str> = stdout.lines().collect();
        assert_eq!(
            lines,
            vec![
                "▸ 2 tool calls · 1 failed (run_command [pytest -q]) — details below",
                "Using tool: run_command",
                "  [pytest -q]",
                "run_command: failed pytest",
            ],
            "header plus the failure's full pair, successes collapsed: {stdout:?}"
        );
    }

    /// C2 property 3: a one-member group emits today's lines, byte for byte.
    #[test]
    fn piped_text_single_call_matches_todays_lines_byte_for_byte() {
        let arguments = serde_json::json!({"path": "notes.md", "content": "hi"});
        let events = vec![
            AgentEvent::tool_requested("workspace_write", arguments, Some(write_effect())),
            AgentEvent::ToolCompleted {
                name: "workspace_write".into(),
                summary: "notes.md written".into(),
            },
            AgentEvent::complete(),
        ];
        let grouped = drain_text_stream(events);
        let mut open = false;
        let ungrouped = format!(
            "{}{}",
            render_agent(
                AgentEvent::tool_requested(
                    "workspace_write",
                    serde_json::json!({"path": "notes.md", "content": "hi"}),
                    Some(write_effect()),
                ),
                RenderFormat::Text,
                &mut open,
            )
            .stdout,
            render_agent(
                AgentEvent::ToolCompleted {
                    name: "workspace_write".into(),
                    summary: "notes.md written".into(),
                },
                RenderFormat::Text,
                &mut open,
            )
            .stdout,
        );
        assert_eq!(
            grouped, ungrouped,
            "a one-member group must keep today's bytes: grouped={grouped:?}"
        );
    }

    /// C2 property 4: NDJSON output is byte-identical to before this slice —
    /// the same event sequence renders event-for-event through the bypass,
    /// with no grouping applied.
    #[test]
    fn ndjson_output_is_byte_identical_with_groups_failures_and_boundaries() {
        let events = mixed_sequence();
        let grouped = drain_ndjson_stream(events.clone());
        let mut open = false;
        let ungrouped: String = events
            .into_iter()
            .map(|event| render_agent(event, RenderFormat::Ndjson, &mut open).stdout)
            .collect();
        assert_eq!(
            grouped, ungrouped,
            "NDJSON must bypass the grouper entirely"
        );
        assert!(
            grouped.lines().count() >= 6,
            "the sequence must contain groups, failures and boundaries: {grouped:?}"
        );
        assert!(
            grouped.contains(r#""event":"tool_requested""#)
                && grouped.contains(r#""event":"tool_completed""#),
            "tool events stay one-per-line on the machine surface: {grouped:?}"
        );
        assert!(
            !grouped.contains("tool calls ·"),
            "no summary line may leak into NDJSON: {grouped:?}"
        );
    }

    /// C2 property 5: text ordering is preserved — the summary appears where
    /// the group's first line would have, relative to surrounding text.
    #[test]
    fn piped_text_summary_keeps_position_relative_to_surrounding_text() {
        let events = vec![
            AgentEvent::assistant_text("before"),
            AgentEvent::tool_requested(
                "workspace_write",
                serde_json::json!({"path": "notes.md", "content": "hi"}),
                Some(write_effect()),
            ),
            AgentEvent::ToolCompleted {
                name: "workspace_write".into(),
                summary: "notes.md written".into(),
            },
            AgentEvent::tool_requested(
                "workspace_write",
                serde_json::json!({"path": "other.md", "content": "hi"}),
                Some(write_effect()),
            ),
            AgentEvent::ToolCompleted {
                name: "workspace_write".into(),
                summary: "other.md written".into(),
            },
            AgentEvent::assistant_text("after"),
            AgentEvent::complete(),
        ];
        let stdout = drain_text_stream(events);
        let before = stdout.find("before").expect("leading text renders");
        let summary = stdout.find("2 tool calls · ok").expect("summary renders");
        let after = stdout.find("after").expect("trailing text renders");
        assert!(
            before < summary && summary < after,
            "the flush happens where the first line would have printed: {stdout:?}"
        );
    }

    /// C2 property 6: a group exceeding the bound flushes rather than
    /// growing — 40 completed calls degrade into chunk summaries.
    #[test]
    fn piped_text_group_exceeding_the_bound_flushes_in_chunks() {
        let mut events = Vec::new();
        for index in 0..40 {
            events.push(AgentEvent::tool_requested(
                "schema_discovery",
                serde_json::json!({"n": index}),
                None,
            ));
            events.push(AgentEvent::ToolCompleted {
                name: "schema_discovery".into(),
                summary: "discovered".into(),
            });
        }
        events.push(AgentEvent::complete());
        let stdout = drain_text_stream(events);
        assert!(
            stdout.contains("32 tool calls · ok"),
            "the bound force-flushes a full chunk: {stdout:?}"
        );
        assert!(
            stdout.contains("8 tool calls · ok"),
            "the remainder flushes as its own chunk: {stdout:?}"
        );
        assert_eq!(
            stdout.lines().count(),
            2,
            "a runaway group degrades into chunk summaries: {stdout:?}"
        );
    }

    /// Property 2 (piped half): `approvals_are_untouched` — a ToolDenied
    /// boundary flushes the open group first and renders the denial line
    /// byte-identical to the ungrouped path, so a call can never collapse
    /// into a group that swallows a consent moment. The prompt/answer half
    /// lives beside the seam that owns it
    /// (`prompt_approval_tests::ask_prompt_keeps_its_bytes_and_answers_under_grouping`);
    /// the bypass-line half beside its seam
    /// (`session_activation_tests::bypass_line_keeps_its_bytes_under_grouping`):
    /// the grouper never sees either string, so grouping cannot touch them.
    #[test]
    fn approvals_are_untouched() {
        let events = vec![
            AgentEvent::tool_requested(
                "workspace_write",
                serde_json::json!({"path": "a.md", "content": "hi"}),
                Some(write_effect()),
            ),
            AgentEvent::ToolCompleted {
                name: "workspace_write".into(),
                summary: "a.md written".into(),
            },
            AgentEvent::tool_requested(
                "workspace_write",
                serde_json::json!({"path": "b.md", "content": "hi"}),
                Some(write_effect()),
            ),
            AgentEvent::ToolCompleted {
                name: "workspace_write".into(),
                summary: "b.md written".into(),
            },
            AgentEvent::ToolDenied {
                name: "run_command".into(),
                reason: "denied".into(),
            },
            AgentEvent::complete(),
        ];
        let stdout = drain_text_stream(events);
        assert!(
            stdout.contains("2 tool calls · ok"),
            "the pre-denial run collapses on its own: {stdout:?}"
        );
        let denied = AgentEvent::ToolDenied {
            name: "run_command".into(),
            reason: "denied".into(),
        };
        let mut open = false;
        let ungrouped = render_agent(denied, RenderFormat::Text, &mut open).stdout;
        assert_eq!(
            ungrouped, "Approval denied for run_command: denied\n",
            "precondition: the denial line under test"
        );
        assert!(
            stdout.contains(&ungrouped),
            "the denial renders byte-identical after the flush: {stdout:?}"
        );
        assert!(
            stdout.find("2 tool calls · ok").expect("summary")
                < stdout.find("Approval denied").expect("denial"),
            "the summary flushes before the boundary: {stdout:?}"
        );
        assert!(
            !stdout.contains("▸ 3 tool calls"),
            "the denied call never joins the collapsed count: {stdout:?}"
        );
    }
}
