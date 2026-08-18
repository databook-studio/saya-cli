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
    let event = terminal_event(event);
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

pub(crate) fn terminal_event(event: AgentEvent) -> TerminalEvent {
    match event {
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
        // rather than fall through to the `unrecognized agent event` catch-all
        // (spec packet-54 decision 4: an event with no renderer previously
        // printed that, and repeating it would be worse than the bug being
        // fixed).
        AgentEvent::KnowledgeLearningSkipped { reason } => {
            TerminalEvent::KnowledgeLearningSkipped { reason }
        }
        // Learning used to fall through to the catch-all below and print
        // `unrecognized agent event` — an error string at the exact moment the
        // product did the thing it is for.
        AgentEvent::KnowledgeProposed { claim } => TerminalEvent::KnowledgeLearned { claim },
        AgentEvent::Complete => TerminalEvent::Complete,
        // AgentEvent is #[non_exhaustive]; a future variant this renderer does not
        // yet understand must not silently terminate the stream (Complete) — surface
        // it as an unimplemented event instead.
        _ => TerminalEvent::NotImplemented {
            feature: "unrecognized agent event".into(),
        },
    }
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
    /// NotImplemented) and renders through the text adapter (spec P1c §5).
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
        // The compact header, the unconfirmed count, and the per-claim lines all
        // reach stdout through the adapter.
        assert!(
            rendered
                .stdout
                .contains("memory supplied · 2 claims (1 unconfirmed)"),
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
        let te = terminal_event(event);
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
    /// NotImplemented) and renders through the text adapter (spec A1 §3).
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
        let te = terminal_event(event);
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
        let te = terminal_event(event);
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
}
