use saya_config::OutputFormat;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderFormat {
    Text,
    Json,
    Ndjson,
}

impl From<crate::cli::FormatArg> for RenderFormat {
    fn from(value: crate::cli::FormatArg) -> Self {
        match value {
            crate::cli::FormatArg::Text => Self::Text,
            crate::cli::FormatArg::Json => Self::Json,
            crate::cli::FormatArg::Ndjson => Self::Ndjson,
        }
    }
}
impl From<OutputFormat> for RenderFormat {
    fn from(value: OutputFormat) -> Self {
        match value {
            OutputFormat::Text => Self::Text,
            OutputFormat::Json => Self::Json,
            OutputFormat::Ndjson => Self::Ndjson,
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum TerminalEvent {
    AssistantText { text: String },
    Result { message: String },
    NotImplemented { feature: String },
    Diagnostic { message: String },
    Error { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rendered {
    pub stdout: String,
    pub stderr: String,
}

pub fn render_event(event: &TerminalEvent, format: RenderFormat) -> Rendered {
    match format {
        RenderFormat::Text => text_event(event),
        RenderFormat::Json | RenderFormat::Ndjson => json_event(event),
    }
}

fn text_event(event: &TerminalEvent) -> Rendered {
    match event {
        TerminalEvent::Diagnostic { message } | TerminalEvent::Error { message } => Rendered {
            stdout: String::new(),
            stderr: format!("{message}\n"),
        },
        TerminalEvent::AssistantText { text } => Rendered {
            stdout: format!("{text}\n"),
            stderr: String::new(),
        },
        TerminalEvent::Result { message } => Rendered {
            stdout: format!("{message}\n"),
            stderr: String::new(),
        },
        TerminalEvent::NotImplemented { feature } => Rendered {
            stdout: format!("Not implemented: {feature}\n"),
            stderr: String::new(),
        },
    }
}

fn json_event(event: &TerminalEvent) -> Rendered {
    let line = serde_json::to_string(event).expect("terminal event is serializable") + "\n";
    match event {
        TerminalEvent::Diagnostic { .. } | TerminalEvent::Error { .. } => Rendered {
            stdout: String::new(),
            stderr: line,
        },
        _ => Rendered {
            stdout: line,
            stderr: String::new(),
        },
    }
}
