use saya_config::OutputFormat;
use saya_types::{QueryResult, SchemaTree};
use serde::Serialize;
mod contract_view;
mod render_contract;
mod render_delta;
mod render_json;
pub use contract_view::{
    ContractClaimView, ContractConflictView, ContractQueueItemView, ContractView,
};
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
    AssistantText {
        text: String,
    },
    ToolRequested {
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },
    ToolCompleted {
        name: String,
        summary: String,
    },
    ToolDenied {
        name: String,
        reason: String,
    },
    Complete,
    Result {
        message: String,
    },
    QueryResult {
        result: QueryResult,
    },
    Schema {
        schema: SchemaTree,
    },
    NotImplemented {
        feature: String,
    },
    Diagnostic {
        message: String,
    },
    Error {
        message: String,
    },
    ContractList {
        contracts: Vec<ContractView>,
    },
    ContractShow {
        contract: ContractView,
    },
    ContractChanged {
        claim_id: String,
        action: String,
        status: String,
    },
    ContractQueue {
        items: Vec<ContractQueueItemView>,
    },
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

pub(super) fn sanitize_terminal(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\n' | '\t' => out.push(c),
            '\x00'..='\x1F' | '\x7F' | '\u{0080}'..='\u{009F}' => {}
            _ => out.push(c),
        }
    }
    out
}

fn text_event(event: &TerminalEvent) -> Rendered {
    let rendered = match event {
        TerminalEvent::Diagnostic { message } | TerminalEvent::Error { message } => Rendered {
            stdout: String::new(),
            stderr: format!("{message}\n"),
        },
        TerminalEvent::AssistantText { text } => render_delta::text(text),
        TerminalEvent::ToolRequested { name, detail } => Rendered {
            stdout: match detail {
                Some(detail) => format!("Using read-only tool: {name}\n  {detail}\n"),
                None => format!("Using read-only tool: {name}\n"),
            },
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
        TerminalEvent::Complete => Rendered {
            stdout: "\n".into(),
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
        TerminalEvent::ContractList { contracts } => render_contract::list(contracts),
        TerminalEvent::ContractShow { contract } => render_contract::show(contract),
        TerminalEvent::ContractChanged {
            claim_id,
            action,
            status,
        } => render_contract::changed(claim_id, action, status),
        TerminalEvent::ContractQueue { items } => render_contract::queue(items),
    };
    Rendered {
        stdout: sanitize_terminal(&rendered.stdout),
        stderr: sanitize_terminal(&rendered.stderr),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_terminal_strips_control_bytes_and_preserves_tabs_and_newlines() {
        let input = "hello\x1b[31mRED\x1b[0m\tworld\n\x1b]0;pwned\x07\r\x7f\u{0080}\u{009f}";
        let sanitized = sanitize_terminal(input);
        assert_eq!(sanitized, "hello[31mRED[0m\tworld\n]0;pwned");
        assert!(!sanitized.contains('\x1b'));
        assert!(!sanitized.contains('\x07'));
        assert!(!sanitized.contains('\r'));
        assert!(!sanitized.contains('\x7f'));
        assert!(!sanitized.contains('\u{0080}'));
        assert!(!sanitized.contains('\u{009f}'));
    }

    #[test]
    fn test_text_render_sanitizes_terminal_control_sequences() {
        let raw_text = "col1\x1b[31mRED\x1b[0m\tcol2\x1b]0;pwned\x07";
        let event = TerminalEvent::QueryResult {
            result: QueryResult {
                columns: vec!["col1".into(), "col2".into()],
                rows: vec![serde_json::json!([raw_text, "ok"])],
                row_count: 1,
                truncated: false,
                executed_sql: "SELECT 1".into(),
            },
        };

        let rendered_text = render_event(&event, RenderFormat::Text);
        assert!(!rendered_text.stdout.contains('\x1b'));
        assert!(!rendered_text.stdout.contains('\x07'));
        assert!(rendered_text.stdout.contains("col1[31mRED[0m"));
        assert!(rendered_text.stdout.contains("pwned"));
        assert!(rendered_text.stdout.contains('\t'));
        assert!(rendered_text.stdout.contains('\n'));

        let rendered_json = render_event(&event, RenderFormat::Json);
        assert!(rendered_json.stdout.contains("\\u001b[31mRED\\u001b[0m"));
        assert!(rendered_json.stdout.contains("\\u001b]0;pwned\\u0007"));
    }

    #[test]
    fn test_assistant_text_and_delta_sanitizes_control_sequences() {
        let raw = "\x1b[31mRED\x1b[0m\x1b]0;pwned\x07";
        let delta_rendered = render_delta::text(raw);
        assert!(!delta_rendered.stdout.contains('\x1b'));
        assert!(!delta_rendered.stdout.contains('\x07'));
        assert_eq!(delta_rendered.stdout, "[31mRED[0m]0;pwned");

        let event = TerminalEvent::AssistantText {
            text: raw.to_string(),
        };
        let text_rendered = render_event(&event, RenderFormat::Text);
        assert!(!text_rendered.stdout.contains('\x1b'));
        assert!(!text_rendered.stdout.contains('\x07'));

        let json_rendered = render_event(&event, RenderFormat::Json);
        assert!(json_rendered.stdout.contains("\\u001b"));
    }
}
