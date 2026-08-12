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
}
