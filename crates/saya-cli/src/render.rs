use saya_config::OutputFormat;
use saya_types::{QueryResult, SchemaTree};
use serde::Serialize;

#[path = "render_json.rs"]
mod render_json;

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

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum TerminalEvent {
    AssistantText { text: String },
    ToolRequested { name: String },
    ToolCompleted { name: String, summary: String },
    ToolDenied { name: String, reason: String },
    Result { message: String },
    QueryResult { result: QueryResult },
    Schema { schema: SchemaTree },
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
        RenderFormat::Json | RenderFormat::Ndjson => render_json::render(event),
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
        TerminalEvent::ToolRequested { name } => Rendered {
            stdout: format!("Using read-only tool: {name}\n"),
            stderr: String::new(),
        },
        TerminalEvent::ToolCompleted { name, summary } => Rendered {
            stdout: format!("{name}: {summary}\n"),
            stderr: String::new(),
        },
        TerminalEvent::ToolDenied { name, reason } => Rendered {
            stdout: format!("Approval denied for {name}: {reason}\n"),
            stderr: String::new(),
        },
        TerminalEvent::Result { message } => Rendered {
            stdout: format!("{message}\n"),
            stderr: String::new(),
        },
        TerminalEvent::QueryResult { result } => Rendered {
            stdout: query_text(result),
            stderr: String::new(),
        },
        TerminalEvent::Schema { schema } => Rendered {
            stdout: format!("{}\n", schema_text(schema)),
            stderr: String::new(),
        },
        TerminalEvent::NotImplemented { feature } => Rendered {
            stdout: format!("Not implemented: {feature}\n"),
            stderr: String::new(),
        },
    }
}

fn query_text(result: &QueryResult) -> String {
    let mut output = result.columns.join("\t");
    if !output.is_empty() {
        output.push('\n');
    }
    for row in &result.rows {
        let value = match row {
            serde_json::Value::Array(values) => values
                .iter()
                .map(display_value)
                .collect::<Vec<_>>()
                .join("\t"),
            value => display_value(value),
        };
        output.push_str(&value);
        output.push('\n');
    }
    if result.truncated {
        output.push_str("[truncated]\n");
    }
    output
}

fn display_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(value) => value.clone(),
        value => value.to_string(),
    }
}

fn schema_text(schema: &SchemaTree) -> String {
    schema
        .databases
        .iter()
        .flat_map(|database| {
            database.schemas.iter().flat_map(move |schema| {
                schema
                    .tables
                    .iter()
                    .map(move |table| format!("{}.{}.{}", database.name, schema.name, table.name))
            })
        })
        .collect::<Vec<_>>()
        .join("\n")
}
