//! Strict, bounded persistence for one receipt-bound diagnostic baseline.
use saya_types::{
    CheckDiagnosticFormat, CodingCheckOutcome, JobId, ManagedCheckoutId, Sha256Digest,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::super::{CheckDiagnostics, CheckReceipt, DiagnosticBaseline, DiagnosticEvidenceError};
use super::{Diagnostic, DiagnosticLocation, DiagnosticSet, DiagnosticSeverity};

pub const DIAGNOSTIC_BASELINE_VERSION: u32 = 1;
pub const MAX_DIAGNOSTIC_BASELINE_BYTES: usize = 64 * 1024;
const MAX_DIAGNOSTICS: usize = 128;
const MAX_MESSAGE_BYTES: usize = 4096;
const MAX_CODE_BYTES: usize = 64;
const MAX_PATH_BYTES: usize = 512;
const MAX_LOCATIONS: usize = 8;
const MAX_LINE: u32 = 10_000_000;
const MAX_COLUMN: u32 = 1_000_000;

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum DiagnosticBaselineWireError {
    #[error("diagnostic baseline is {0} bytes, over its limit")]
    Oversize(usize),
    #[error("diagnostic baseline is malformed or has an unsupported version")]
    Malformed,
    #[error("diagnostic baseline digest does not match its payload")]
    DigestMismatch,
    #[error("diagnostic baseline identity is invalid")]
    IdentityMismatch,
}

/// Immutable identity needed to compare a later receipt with the baseline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BaselineIdentity {
    pub(crate) job_id: JobId,
    pub(crate) checkout_id: ManagedCheckoutId,
    pub(crate) profile_id: String,
    pub(crate) source_sha256: Sha256Digest,
    pub(crate) comparison_sha256: Sha256Digest,
    pub(crate) outcome: CodingCheckOutcome,
    pub(crate) output_complete: bool,
}

/// A validated baseline that contains no authority and can only be used for
/// diagnostic comparison.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedDiagnosticBaseline {
    pub(crate) key: BaselineIdentity,
    pub(crate) format: CheckDiagnosticFormat,
    pub(crate) parser_revision: u32,
    pub(crate) diagnostics: DiagnosticSet,
}

impl PersistedDiagnosticBaseline {
    pub(crate) fn from_proof(
        baseline: &DiagnosticBaseline<'_>,
    ) -> Result<Self, DiagnosticEvidenceError> {
        let evidence = baseline.evidence();
        let diagnostics = evidence
            .diagnostics()
            .map_err(|_| DiagnosticEvidenceError::Incomplete)?
            .map(redacted_diagnostics);
        Ok(Self {
            key: BaselineIdentity::from_receipt(evidence),
            format: evidence.format(),
            parser_revision: diagnostics.parser_revision(),
            diagnostics,
        })
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, DiagnosticBaselineWireError> {
        let payload = WirePayload::from_record(self);
        let payload_bytes =
            serde_json::to_vec(&payload).map_err(|_| DiagnosticBaselineWireError::Malformed)?;
        let digest = digest(&payload_bytes);
        let envelope = WireEnvelope {
            version: DIAGNOSTIC_BASELINE_VERSION,
            payload,
            payload_sha256: digest,
        };
        let bytes =
            serde_json::to_vec(&envelope).map_err(|_| DiagnosticBaselineWireError::Malformed)?;
        if bytes.len() > MAX_DIAGNOSTIC_BASELINE_BYTES {
            return Err(DiagnosticBaselineWireError::Oversize(bytes.len()));
        }
        Ok(bytes)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, DiagnosticBaselineWireError> {
        if bytes.len() > MAX_DIAGNOSTIC_BASELINE_BYTES {
            return Err(DiagnosticBaselineWireError::Oversize(bytes.len()));
        }
        let envelope: WireEnvelope =
            serde_json::from_slice(bytes).map_err(|_| DiagnosticBaselineWireError::Malformed)?;
        if envelope.version != DIAGNOSTIC_BASELINE_VERSION {
            return Err(DiagnosticBaselineWireError::Malformed);
        }
        let payload_bytes = serde_json::to_vec(&envelope.payload)
            .map_err(|_| DiagnosticBaselineWireError::Malformed)?;
        if digest(&payload_bytes) != envelope.payload_sha256 {
            return Err(DiagnosticBaselineWireError::DigestMismatch);
        }
        Self::from_wire(envelope.payload)
    }

    fn from_wire(payload: WirePayload) -> Result<Self, DiagnosticBaselineWireError> {
        let diagnostics = payload
            .diagnostics
            .into_iter()
            .map(WireDiagnostic::into_diagnostic)
            .collect::<Result<Vec<_>, _>>()?;
        if diagnostics.len() > MAX_DIAGNOSTICS {
            return Err(DiagnosticBaselineWireError::Malformed);
        }
        if diagnostics.windows(2).any(|pair| pair[0] > pair[1]) {
            return Err(DiagnosticBaselineWireError::Malformed);
        }
        let diagnostics = DiagnosticSet::new(diagnostics);
        if diagnostics.parser_revision() != payload.parser_revision {
            return Err(DiagnosticBaselineWireError::Malformed);
        }
        let key = payload.identity.validate()?;
        Ok(Self {
            key,
            format: payload.format,
            parser_revision: payload.parser_revision,
            diagnostics,
        })
    }
}

impl BaselineIdentity {
    fn from_receipt(evidence: &CheckDiagnostics) -> Self {
        Self::from_parts(evidence.receipt())
    }

    pub(crate) fn from_parts(receipt: &CheckReceipt) -> Self {
        let wire = receipt.wire_receipt();
        Self {
            job_id: wire.job_id().clone(),
            checkout_id: receipt.checkout_id().clone(),
            profile_id: receipt.profile_id().to_owned(),
            source_sha256: wire.source_sha256().clone(),
            comparison_sha256: receipt_identity_digest(receipt),
            outcome: receipt.outcome(),
            output_complete: receipt.output().complete(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireEnvelope {
    version: u32,
    payload: WirePayload,
    payload_sha256: Sha256Digest,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WirePayload {
    identity: BaselineIdentity,
    format: CheckDiagnosticFormat,
    parser_revision: u32,
    diagnostics: Vec<WireDiagnostic>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WireDiagnostic {
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
    fn from_record(record: &PersistedDiagnosticBaseline) -> Self {
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

impl BaselineIdentity {
    fn validate(self) -> Result<Self, DiagnosticBaselineWireError> {
        if !valid_reference(&self.profile_id)
            || !self.output_complete
            || !matches!(self.outcome, CodingCheckOutcome::KnownExit { .. })
        {
            return Err(DiagnosticBaselineWireError::IdentityMismatch);
        }
        Ok(self)
    }
}

fn receipt_identity_digest(receipt: &CheckReceipt) -> Sha256Digest {
    let bytes = serde_json::to_vec(&(
        receipt.wire.job_id(),
        receipt.checkout_id.as_str(),
        receipt.profile_id.as_str(),
        receipt.program.as_str(),
        receipt.toolchain.as_deref(),
        receipt.wire.command_id(),
        receipt.wire.argv_sha256(),
        receipt.wire.environment_profile(),
        receipt.lineage.checkout.as_str(),
        receipt.lineage.path.as_str(),
        receipt.lineage.repository.as_str(),
        receipt.lineage.snapshot.as_str(),
        receipt.lineage.snapshot_sha256.as_str(),
        receipt.lineage.plan_sha256.as_str(),
        receipt.lineage.base.as_str(),
        receipt.lineage.agent.as_ref().map(|agent| agent.as_str()),
        receipt.lineage.policy.as_str(),
        receipt.lineage.epoch,
    ))
    .expect("receipt identity serializes");
    let digest = Sha256::digest(bytes);
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    Sha256Digest::parse(&hex).expect("sha256 encoding is valid")
}

pub(crate) fn receipt_matches(receipt: &CheckReceipt, baseline: &BaselineIdentity) -> bool {
    receipt.checkout_id == baseline.checkout_id
        && receipt.profile_id == baseline.profile_id
        && receipt.wire.job_id() == &baseline.job_id
        && receipt_identity_digest(receipt) == baseline.comparison_sha256
}

pub(crate) fn redacted_diagnostics(diagnostics: &DiagnosticSet) -> DiagnosticSet {
    DiagnosticSet::new(
        diagnostics
            .entries()
            .iter()
            .map(|diagnostic| {
                Diagnostic::new(
                    diagnostic.severity(),
                    diagnostic.code().map(str::to_owned),
                    saya_types::redact(diagnostic.message()),
                    diagnostic.locations().to_vec(),
                )
            })
            .collect(),
    )
}

fn valid_reference(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
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

    fn into_diagnostic(self) -> Result<Diagnostic, DiagnosticBaselineWireError> {
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
        let code = self.code.filter(|code| {
            !code.is_empty()
                && code.len() <= MAX_CODE_BYTES
                && code
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b':' | b'-'))
        });
        if self.code.is_some() && code.is_none() {
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

fn digest(bytes: &[u8]) -> Sha256Digest {
    let digest = Sha256::digest(bytes);
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    Sha256Digest::parse(&hex).expect("sha256 encoding is valid")
}
