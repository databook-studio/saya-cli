use std::path::Path;

use serde_json::Value;

use saya_types::CheckDiagnosticFormat;

use super::{
    location::primary_locations,
    types::{Diagnostic, DiagnosticError, DiagnosticSet, DiagnosticSeverity},
};

const MAX_INPUT_BYTES: usize = 64 * 1024;
const MAX_RECORD_BYTES: usize = 32 * 1024;
const MAX_DIAGNOSTICS: usize = 128;
const MAX_MESSAGE_BYTES: usize = 4096;
const MAX_CODE_BYTES: usize = 64;

pub(in crate::coding::checks) fn parse(
    format: CheckDiagnosticFormat,
    bytes: &[u8],
    candidate_root: &Path,
) -> Result<DiagnosticSet, DiagnosticError> {
    if bytes.len() > MAX_INPUT_BYTES {
        return Err(DiagnosticError::InputTooLarge);
    }
    let input = std::str::from_utf8(bytes).map_err(|_| DiagnosticError::InvalidUtf8)?;
    let mut diagnostics = Vec::new();
    let mut cargo_finished = false;
    for (index, line) in input.split_terminator('\n').enumerate() {
        let line_number = index + 1;
        if line.len() > MAX_RECORD_BYTES {
            return Err(DiagnosticError::RecordTooLarge { line: line_number });
        }
        if cargo_finished {
            return Err(DiagnosticError::InvalidRecord { line: line_number });
        }
        let record: Value = serde_json::from_str(line)
            .map_err(|_| DiagnosticError::InvalidRecord { line: line_number })?;
        let (diagnostic, finished) = parse_record(format, &record, candidate_root, line_number)?;
        cargo_finished = finished;
        if let Some(diagnostic) = diagnostic {
            if diagnostics.len() == MAX_DIAGNOSTICS {
                return Err(DiagnosticError::TooManyDiagnostics);
            }
            diagnostics.push(diagnostic);
        }
    }
    diagnostics.sort();
    Ok(DiagnosticSet::new(diagnostics))
}

fn parse_record(
    format: CheckDiagnosticFormat,
    record: &Value,
    candidate_root: &Path,
    line: usize,
) -> Result<(Option<Diagnostic>, bool), DiagnosticError> {
    let object = record
        .as_object()
        .ok_or(DiagnosticError::InvalidRecord { line })?;
    match format {
        CheckDiagnosticFormat::RustcJson => {
            if object.contains_key("reason") {
                return Err(DiagnosticError::UnsupportedFormat);
            }
            let kind = object
                .get("$message_type")
                .and_then(Value::as_str)
                .ok_or(DiagnosticError::InvalidRecord { line })?;
            match kind {
                "diagnostic" => Ok((
                    Some(parse_diagnostic(record, candidate_root, line, true)?),
                    false,
                )),
                "artifact" => {
                    require_string(object.get("artifact"), line)?;
                    require_string(object.get("emit"), line)?;
                    Ok((None, false))
                }
                "future_incompat" => Err(DiagnosticError::UnsupportedFormat),
                _ => Err(DiagnosticError::InvalidRecord { line }),
            }
        }
        CheckDiagnosticFormat::CargoJson => {
            if object.contains_key("$message_type") {
                return Err(DiagnosticError::UnsupportedFormat);
            }
            let reason = object
                .get("reason")
                .and_then(Value::as_str)
                .ok_or(DiagnosticError::UnsupportedFormat)?;
            match reason {
                "compiler-message" => {
                    let message = object
                        .get("message")
                        .ok_or(DiagnosticError::InvalidRecord { line })?;
                    Ok((
                        Some(parse_diagnostic(message, candidate_root, line, false)?),
                        false,
                    ))
                }
                "compiler-artifact" => {
                    require_string(object.get("package_id"), line)?;
                    require_object(object.get("target"), line)?;
                    require_object(object.get("profile"), line)?;
                    require_array(object.get("filenames"), line)?;
                    require_bool(object.get("fresh"), line)?;
                    Ok((None, false))
                }
                "build-script-executed" => {
                    require_string(object.get("package_id"), line)?;
                    require_array(object.get("linked_libs"), line)?;
                    require_array(object.get("cfgs"), line)?;
                    require_array(object.get("env"), line)?;
                    require_string(object.get("out_dir"), line)?;
                    Ok((None, false))
                }
                "build-finished" => {
                    require_bool(object.get("success"), line)?;
                    Ok((None, true))
                }
                _ => Err(DiagnosticError::InvalidRecord { line }),
            }
        }
        _ => Err(DiagnosticError::UnsupportedFormat),
    }
}

fn parse_diagnostic(
    value: &Value,
    candidate_root: &Path,
    line: usize,
    require_message_type: bool,
) -> Result<Diagnostic, DiagnosticError> {
    let object = value
        .as_object()
        .ok_or(DiagnosticError::InvalidDiagnostic { line })?;
    let message_type = object.get("$message_type").and_then(Value::as_str);
    if (require_message_type && message_type != Some("diagnostic"))
        || message_type.is_some_and(|kind| kind != "diagnostic")
    {
        return Err(DiagnosticError::UnsupportedFormat);
    }
    let message = object
        .get("message")
        .and_then(Value::as_str)
        .filter(|message| {
            message.len() <= MAX_MESSAGE_BYTES
                && !message
                    .chars()
                    .any(|character| character.is_control() && !matches!(character, '\n' | '\t'))
        })
        .ok_or(DiagnosticError::InvalidDiagnostic { line })?;
    let severity = parse_severity(object.get("level").and_then(Value::as_str))
        .ok_or(DiagnosticError::InvalidDiagnostic { line })?;
    let code = parse_code(object.get("code"), line)?;
    let spans = object
        .get("spans")
        .ok_or(DiagnosticError::InvalidDiagnostic { line })?;
    let locations = primary_locations(spans, candidate_root, line)?;
    Ok(Diagnostic::new(
        severity,
        code,
        message.to_owned(),
        locations,
    ))
}

fn parse_severity(value: Option<&str>) -> Option<DiagnosticSeverity> {
    match value? {
        "error" => Some(DiagnosticSeverity::Error),
        "warning" => Some(DiagnosticSeverity::Warning),
        "note" => Some(DiagnosticSeverity::Note),
        "help" => Some(DiagnosticSeverity::Help),
        "failure-note" => Some(DiagnosticSeverity::FailureNote),
        "error: internal compiler error" => Some(DiagnosticSeverity::InternalCompilerError),
        _ => None,
    }
}

fn parse_code(value: Option<&Value>, line: usize) -> Result<Option<String>, DiagnosticError> {
    let Some(value) = value else {
        return Err(DiagnosticError::InvalidDiagnostic { line });
    };
    if value.is_null() {
        return Ok(None);
    }
    let code = value
        .as_object()
        .and_then(|object| object.get("code"))
        .and_then(Value::as_str)
        .filter(|code| {
            !code.is_empty()
                && code.len() <= MAX_CODE_BYTES
                && code
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b':' | b'-'))
        })
        .ok_or(DiagnosticError::InvalidDiagnostic { line })?;
    Ok(Some(code.to_owned()))
}

fn require_string(value: Option<&Value>, line: usize) -> Result<(), DiagnosticError> {
    if value.and_then(Value::as_str).is_some() {
        Ok(())
    } else {
        Err(DiagnosticError::InvalidRecord { line })
    }
}

fn require_object(value: Option<&Value>, line: usize) -> Result<(), DiagnosticError> {
    if value.and_then(Value::as_object).is_some() {
        Ok(())
    } else {
        Err(DiagnosticError::InvalidRecord { line })
    }
}

fn require_array(value: Option<&Value>, line: usize) -> Result<(), DiagnosticError> {
    if value.and_then(Value::as_array).is_some() {
        Ok(())
    } else {
        Err(DiagnosticError::InvalidRecord { line })
    }
}

fn require_bool(value: Option<&Value>, line: usize) -> Result<(), DiagnosticError> {
    if value.and_then(Value::as_bool).is_some() {
        Ok(())
    } else {
        Err(DiagnosticError::InvalidRecord { line })
    }
}
