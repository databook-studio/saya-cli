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
}
impl TerminalSink {
    pub(crate) fn new(format: RenderFormat) -> Self {
        Self {
            format,
            text_open: Mutex::new(false),
        }
    }
}

#[async_trait]
impl AgentEventSink for TerminalSink {
    async fn emit(&self, event: AgentEvent) {
        let rendered = render_agent(
            event,
            self.format,
            &mut self.text_open.lock().expect("terminal state"),
        );
        print!("{}", rendered.stdout);
        eprint!("{}", rendered.stderr);
        let _ = io::stdout().flush();
        let _ = io::stderr().flush();
    }
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
        AgentEvent::ToolRequested { name, arguments } => {
            let detail = crate::agent::tools::tool_call_detail(&name, &arguments);
            TerminalEvent::ToolRequested { name, detail }
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
        // Learning used to fall through to the catch-all below and print
        // `unrecognized agent event` — an error string at the exact moment the
        // product did the thing it is for.
        AgentEvent::KnowledgeProposed { claim } => TerminalEvent::KnowledgeLearned { claim },
        // Progress only: the TUI labels its spinner with this, and a pipe has
        // no spinner to label. Dropped rather than rendered — not forgotten,
        // which is what the catch-all would make of it.
        AgentEvent::KnowledgeLearningStarted => return None,
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
        AgentEvent::Complete => TerminalEvent::Complete,
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
        assert_eq!(
            render_agent(
                AgentEvent::tool_requested("schema", serde_json::Value::Null),
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
}
