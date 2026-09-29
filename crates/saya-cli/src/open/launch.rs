//! Presenting the staged source and launching its session: the preview
//! printed before the TUI opens (and the whole output when non-TTY), and the
//! launch itself — a `Cli` with the generated connections file and profile
//! overrides, exactly as `saya demo` launches.

use crate::cli::{Cli, GlobalOptions};
use saya_harness::file_source::{InferredType, SourceFormat};
use std::path::Path;

use super::{stage::StagedSession, typed::TypedReport};

fn type_name(inferred: InferredType) -> &'static str {
    match inferred {
        InferredType::Integer => "integer",
        InferredType::Decimal => "decimal",
        InferredType::Boolean => "boolean",
        InferredType::Date => "date",
        InferredType::Timestamp => "timestamp",
        InferredType::Text => "text",
    }
}

fn delimiter_label(byte: u8) -> String {
    match byte {
        b'\t' => "TAB".to_owned(),
        _ => (byte as char).to_string(),
    }
}

/// The preview message: identity, shape, inferred columns, the staged path,
/// and the launch command. `typed` is `Some` when a typed copy was built.
pub(super) fn render_preview(
    session: &StagedSession,
    connections: &Path,
    profile: &str,
    typed: Option<&TypedReport>,
) -> String {
    let state = if session.reused {
        "reused existing snapshot"
    } else {
        "staged now"
    };
    let mut lines = vec![
        format!("File: {} ({state})", session.file_name),
        format!("SHA-256: {}", &session.sha256[..12]),
        format!("Size: {}", super::human_bytes(session.bytes)),
        format!(
            "Rows: {} · Columns: {}",
            session.rows,
            session.preview.columns.len()
        ),
    ];
    match session.format {
        SourceFormat::Parquet => lines.push("Format: parquet".to_owned()),
        SourceFormat::Csv => lines.push(format!(
            "Delimiter: {} · Header row: {}",
            delimiter_label(session.preview.delimiter),
            if session.preview.header { "yes" } else { "no" }
        )),
    }
    lines.push("Columns:".to_owned());
    for column in &session.preview.columns {
        lines.push(format!(
            "  {}: {} ({} nulls)",
            column.name,
            type_name(column.inferred),
            column.null_count
        ));
    }
    lines.push(format!(
        "Staged: {} ({})",
        session.db_path.display(),
        super::format_time(session.staged_unix_ms)
    ));
    match session.format {
        SourceFormat::Parquet => {
            lines.push("Columns keep their native Parquet types.".to_owned());
        }
        SourceFormat::Csv => {
            lines.push("Stored as text columns; use --typed for a typed copy.".to_owned());
        }
    }
    if let Some(typed) = typed {
        lines.push(format!(
            "Typed copy: {} created{}",
            typed.table,
            cast_summary(typed)
        ));
        lines.push(failure_summary(typed));
    }
    lines.push("Open it read-only:".to_owned());
    lines.push(format!(
        "  saya --connections {} --profile {profile}",
        connections.display()
    ));
    lines.join("\n")
}

fn cast_summary(typed: &TypedReport) -> String {
    let casts = typed
        .cast_columns
        .iter()
        .map(|(name, label)| format!("{name}: {label}"))
        .collect::<Vec<_>>()
        .join(", ");
    let text = typed.text_columns.join(", ");
    match (casts.is_empty(), text.is_empty()) {
        (true, true) => String::new(),
        (false, true) => format!(" ({casts})"),
        (true, false) => format!(" (nothing to cast; kept as text: {text})"),
        (false, false) => format!(" ({casts}; kept as text: {text})"),
    }
}

fn failure_summary(typed: &TypedReport) -> String {
    if typed.failures.is_empty() {
        return "Cast failures: none".to_owned();
    }
    let mut lines = vec!["Cast failures:".to_owned()];
    for (name, failed, total) in &typed.failures {
        lines.push(format!(
            "  {name}: {failed} of {total} values did not cast (kept NULL)"
        ));
    }
    lines.join("\n")
}

pub(super) fn launch(
    cli: &Cli,
    connections: &Path,
    profile: &str,
) -> Result<i32, Box<dyn std::error::Error>> {
    let session = Cli {
        options: GlobalOptions {
            connections: Some(connections.to_path_buf()),
            profile: Some(profile.to_owned()),
            ..cli.options.clone()
        },
        command: None,
    };
    crate::interactive::run(session)
}
