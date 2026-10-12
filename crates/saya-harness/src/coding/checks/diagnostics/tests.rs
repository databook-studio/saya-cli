//! Shared fixtures, helpers, and rejection macros for the diagnostics parser tests.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::{
    parse::parse,
    types::{DiagnosticError, DiagnosticSeverity},
};
use saya_types::CheckDiagnosticFormat;

mod formats;
mod limits;
mod locations;
mod redaction;

const RUSTC: CheckDiagnosticFormat = CheckDiagnosticFormat::RustcJson;
const CARGO: CheckDiagnosticFormat = CheckDiagnosticFormat::CargoJson;

#[cfg(windows)]
fn fixture_root() -> PathBuf {
    PathBuf::from("C:/saya-parser-fixture")
}

#[cfg(not(windows))]
fn fixture_root() -> PathBuf {
    PathBuf::from("/saya-parser-fixture")
}

fn span(file: &str, line: u64, column: u64, primary: bool) -> Value {
    json!({
        "file_name": file,
        "line_start": line,
        "line_end": line,
        "column_start": column,
        "column_end": column + 1,
        "is_primary": primary,
    })
}

fn diagnostic(message: &str, level: &str, code: Value, spans: Vec<Value>) -> Value {
    json!({
        "$message_type": "diagnostic",
        "message": message,
        "code": code,
        "level": level,
        "spans": spans,
        "children": [{"message": "child-secret-must-not-be-retained"}],
        "rendered": "rendered-secret-must-not-be-retained",
        "unknown_extra_field": true,
    })
}

fn line(value: &Value) -> String {
    serde_json::to_string(value).unwrap()
}

fn parse_at(
    format: CheckDiagnosticFormat,
    input: &str,
    root: &Path,
) -> Result<super::types::DiagnosticSet, DiagnosticError> {
    parse(format, input.as_bytes(), root)
}

macro_rules! rejects {
    ($format:expr, $input:expr, $root:expr, $error:expr) => {
        assert_eq!(parse($format, $input, $root).unwrap_err(), $error)
    };
}

macro_rules! rejects_text {
    ($format:expr, $input:expr, $root:expr, $error:expr) => {
        rejects!($format, $input.as_bytes(), $root, $error)
    };
}

macro_rules! rejects_value {
    ($format:expr, $value:expr, $root:expr, $error:expr) => {
        rejects_text!($format, &line($value), $root, $error)
    };
}

pub(super) use {rejects, rejects_text, rejects_value};
