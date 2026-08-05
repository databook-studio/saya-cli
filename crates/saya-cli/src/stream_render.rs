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
        AgentEvent::ToolRequested { name } => TerminalEvent::ToolRequested { name },
        AgentEvent::ToolCompleted { name, summary } => {
            TerminalEvent::ToolCompleted { name, summary }
        }
        AgentEvent::ToolDenied { name, reason } => TerminalEvent::ToolDenied { name, reason },
        AgentEvent::Complete => TerminalEvent::Complete,
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
                AgentEvent::AssistantText {
                    text: "thinking".into()
                },
                RenderFormat::Text,
                &mut open
            )
            .stdout,
            "thinking"
        );
        assert_eq!(
            render_agent(
                AgentEvent::ToolRequested {
                    name: "schema".into()
                },
                RenderFormat::Text,
                &mut open
            )
            .stdout,
            "\nUsing read-only tool: schema\n"
        );
        assert_eq!(
            render_agent(AgentEvent::Complete, RenderFormat::Text, &mut open).stdout,
            ""
        );
    }
    #[test]
    fn ndjson_keeps_delta_and_complete_envelopes() {
        let mut open = false;
        assert_eq!(
            render_agent(
                AgentEvent::AssistantText { text: "x".into() },
                RenderFormat::Ndjson,
                &mut open
            )
            .stdout,
            "{\"event\":\"assistant_text\",\"text\":\"x\"}\n"
        );
        assert_eq!(
            render_agent(AgentEvent::Complete, RenderFormat::Ndjson, &mut open).stdout,
            "{\"event\":\"complete\"}\n"
        );
    }
    #[test]
    fn text_complete_closes_an_open_delta_line_once() {
        let mut open = false;
        let _ = render_agent(
            AgentEvent::AssistantText {
                text: "done".into(),
            },
            RenderFormat::Text,
            &mut open,
        );
        assert_eq!(
            render_agent(AgentEvent::Complete, RenderFormat::Text, &mut open).stdout,
            "\n"
        );
    }
}
