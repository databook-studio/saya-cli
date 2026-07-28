use crate::{RenderFormat, TerminalEvent, render_event};
use async_trait::async_trait;
use saya_agent::{AgentEvent, AgentEventSink};
use std::io::{self, Write};

pub(crate) struct TerminalSink {
    format: RenderFormat,
}
impl TerminalSink {
    pub(crate) fn new(format: RenderFormat) -> Self {
        Self { format }
    }
}

#[async_trait]
impl AgentEventSink for TerminalSink {
    async fn emit(&self, event: AgentEvent) {
        let Some(event) = terminal_event(event) else {
            return;
        };
        let rendered = render_event(&event, self.format);
        print!("{}", rendered.stdout);
        eprint!("{}", rendered.stderr);
        let _ = io::stdout().flush();
        let _ = io::stderr().flush();
    }
}

pub(crate) fn terminal_event(event: AgentEvent) -> Option<TerminalEvent> {
    match event {
        AgentEvent::AssistantText { text } => Some(TerminalEvent::AssistantText { text }),
        AgentEvent::ToolRequested { name } => Some(TerminalEvent::ToolRequested { name }),
        AgentEvent::ToolCompleted { name, summary } => {
            Some(TerminalEvent::ToolCompleted { name, summary })
        }
        AgentEvent::ToolDenied { name, reason } => Some(TerminalEvent::ToolDenied { name, reason }),
        AgentEvent::Complete => Some(TerminalEvent::Complete),
    }
}
