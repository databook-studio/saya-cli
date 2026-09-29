//! The parse contract that shares snapshot identity with the content hash:
//! the canonical contract string (format, effective delimiter, header flag,
//! table name, typed flag), the short digest a snapshot directory appends to
//! the content-hash prefix, and the table name read back out of the string.

use saya_harness::file_source::{InferredType, Preview, SourceFormat};
use sha2::{Digest, Sha256};

/// The canonical parse contract of one staged snapshot. CSV carries the
/// effective delimiter and header flag — the resolved values, so an explicit
/// option naming the sniffed default is the same contract; Parquet has
/// neither. The table name and typed flag close the identity, so a renamed
/// copy of identical bytes and a `--typed` open are separate snapshots.
pub(super) fn canonical(
    format: SourceFormat,
    preview: &Preview,
    table: &str,
    typed: bool,
) -> String {
    match format {
        SourceFormat::Csv => format!(
            "csv;delimiter={};header={};table={table};typed={}",
            preview.delimiter,
            flag(preview.header),
            flag(typed),
        ),
        SourceFormat::Parquet => format!("parquet;table={table};typed={}", flag(typed)),
    }
}

fn flag(set: bool) -> &'static str {
    if set { "1" } else { "0" }
}

/// The 8-hex digest the snapshot directory name appends to the content-hash
/// prefix.
pub(super) fn digest(contract: &str) -> String {
    Sha256::digest(contract.as_bytes())
        .iter()
        .take(4)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The contract's table name field.
pub(super) fn table_of(contract: &str) -> Option<&str> {
    contract
        .split(';')
        .find_map(|field| field.strip_prefix("table="))
        .filter(|table| !table.is_empty())
}

/// The preview's type labels, shared with the stored preview's read-back so
/// the two spellings cannot drift.
pub(super) fn inferred_label(inferred: InferredType) -> &'static str {
    match inferred {
        InferredType::Integer => "integer",
        InferredType::Decimal => "decimal",
        InferredType::Boolean => "boolean",
        InferredType::Date => "date",
        InferredType::Timestamp => "timestamp",
        InferredType::Text => "text",
    }
}

pub(super) fn inferred_from_label(label: &str) -> Option<InferredType> {
    match label {
        "integer" => Some(InferredType::Integer),
        "decimal" => Some(InferredType::Decimal),
        "boolean" => Some(InferredType::Boolean),
        "date" => Some(InferredType::Date),
        "timestamp" => Some(InferredType::Timestamp),
        "text" => Some(InferredType::Text),
        _ => None,
    }
}
