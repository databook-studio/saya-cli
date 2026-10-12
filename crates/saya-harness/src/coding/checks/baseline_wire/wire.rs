//! Wire encoding: envelope/payload serde types, per-record validation, and severity names.

use saya_types::{CheckDiagnosticFormat, Sha256Digest};
use serde::{Deserialize, Serialize};

use super::{
    BaselineIdentity, Diagnostic, DiagnosticBaselineWireError, DiagnosticLocation,
    DiagnosticSeverity,
};

const MAX_MESSAGE_BYTES: usize = 4096;
const MAX_CODE_BYTES: usize = 64;
const MAX_PATH_BYTES: usize = 512;
const MAX_LOCATIONS: usize = 8;
const MAX_LINE: u32 = 10_000_000;
const MAX_COLUMN: u32 = 1_000_000;

#[derive(Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WireEnvelope {
    pub(super) version: u32,
    pub(super) payload: WirePayload,
    pub(super) payload_sha256: Sha256Digest,
}

/// Borrows the payload bytes exactly as transmitted so the digest covers the
/// wire representation rather than a re-serialization of parsed content.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawEnvelope<'a> {
    pub(super) version: u32,
    #[serde(borrow)]
    pub(super) payload: &'a serde_json::value::RawValue,
    pub(super) payload_sha256: Sha256Digest,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WirePayload {
    pub(super) identity: BaselineIdentity,
    pub(super) format: CheckDiagnosticFormat,
    pub(super) parser_revision: u32,
    pub(super) diagnostics: Vec<WireDiagnostic>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WireDiagnostic {
    severity: String,
    code: Option<String>,
    message: String,
    locations: Vec<WireLocation>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireLocation {
    path: String,
    line_start: u32,
    line_end: u32,
    column_start: u32,
    column_end: u32,
}

impl WirePayload {
    pub(super) fn from_record(record: &super::PersistedDiagnosticBaseline) -> Self {
        Self {
            identity: record.key.clone(),
            format: record.format,
            parser_revision: record.parser_revision,
            diagnostics: record
                .diagnostics
                .entries()
                .iter()
                .map(WireDiagnostic::from_diagnostic)
                .collect(),
        }
    }
}

impl WireDiagnostic {
    fn from_diagnostic(diagnostic: &Diagnostic) -> Self {
        Self {
            severity: severity_name(diagnostic.severity()).to_owned(),
            code: diagnostic.code().map(str::to_owned),
            message: diagnostic.message().to_owned(),
            locations: diagnostic
                .locations()
                .iter()
                .map(|location| WireLocation {
                    path: location.path().to_owned(),
                    line_start: location.line_start(),
                    line_end: location.line_end(),
                    column_start: location.column_start(),
                    column_end: location.column_end(),
                })
                .collect(),
        }
    }

    pub(super) fn into_diagnostic(self) -> Result<Diagnostic, DiagnosticBaselineWireError> {
        let severity = parse_severity(&self.severity)?;
        if self.message.is_empty()
            || self.message.len() > MAX_MESSAGE_BYTES
            || self
                .message
                .chars()
                .any(|character| character.is_control() && !matches!(character, '\n' | '\t'))
            || self.locations.len() > MAX_LOCATIONS
        {
            return Err(DiagnosticBaselineWireError::Malformed);
        }
        let had_code = self.code.is_some();
        let code = self.code.filter(|code| {
            !code.is_empty()
                && code.len() <= MAX_CODE_BYTES
                && code
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b':' | b'-'))
        });
        if had_code && code.is_none() {
            return Err(DiagnosticBaselineWireError::Malformed);
        }
        let locations = self
            .locations
            .into_iter()
            .map(WireLocation::into_location)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Diagnostic::new(severity, code, self.message, locations))
    }
}

impl WireLocation {
    fn into_location(self) -> Result<DiagnosticLocation, DiagnosticBaselineWireError> {
        if self.path.is_empty()
            || self.path.len() > MAX_PATH_BYTES
            || self.path.contains('\\')
            || self.path.chars().any(char::is_control)
            || self
                .path
                .split('/')
                .any(|part| part.is_empty() || matches!(part, "." | ".."))
            || !(1..=MAX_LINE).contains(&self.line_start)
            || !(1..=MAX_LINE).contains(&self.line_end)
            || !(1..=MAX_COLUMN).contains(&self.column_start)
            || !(1..=MAX_COLUMN).contains(&self.column_end)
            || (self.line_start, self.column_start) > (self.line_end, self.column_end)
        {
            return Err(DiagnosticBaselineWireError::Malformed);
        }
        Ok(DiagnosticLocation::new(
            self.path,
            self.line_start,
            self.line_end,
            self.column_start,
            self.column_end,
        ))
    }
}

fn severity_name(severity: DiagnosticSeverity) -> &'static str {
    match severity {
        DiagnosticSeverity::Error => "error",
        DiagnosticSeverity::Warning => "warning",
        DiagnosticSeverity::Note => "note",
        DiagnosticSeverity::Help => "help",
        DiagnosticSeverity::FailureNote => "failure-note",
        DiagnosticSeverity::InternalCompilerError => "error: internal compiler error",
    }
}

fn parse_severity(value: &str) -> Result<DiagnosticSeverity, DiagnosticBaselineWireError> {
    match value {
        "error" => Ok(DiagnosticSeverity::Error),
        "warning" => Ok(DiagnosticSeverity::Warning),
        "note" => Ok(DiagnosticSeverity::Note),
        "help" => Ok(DiagnosticSeverity::Help),
        "failure-note" => Ok(DiagnosticSeverity::FailureNote),
        "error: internal compiler error" => Ok(DiagnosticSeverity::InternalCompilerError),
        _ => Err(DiagnosticBaselineWireError::Malformed),
    }
}
