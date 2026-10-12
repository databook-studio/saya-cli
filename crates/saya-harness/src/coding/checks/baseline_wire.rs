//! Strict, bounded persistence for one receipt-bound diagnostic baseline.
use saya_types::{CheckDiagnosticFormat, Sha256Digest};
use sha2::{Digest, Sha256};

use super::super::{DiagnosticBaseline, DiagnosticEvidenceError};
use super::{Diagnostic, DiagnosticLocation, DiagnosticSet, DiagnosticSeverity};

// This module is loaded through `#[path = "../baseline_wire.rs"]` from
// `diagnostics/mod.rs`, so it does not own a subdirectory: the child module
// files need explicit paths to live in `baseline_wire/`.
#[path = "baseline_wire/identity.rs"]
mod identity;

#[path = "baseline_wire/wire.rs"]
mod wire;

pub(crate) use identity::{BaselineIdentity, receipt_matches};
use wire::{RawEnvelope, WireDiagnostic, WireEnvelope, WirePayload};

pub const DIAGNOSTIC_BASELINE_VERSION: u32 = 1;
pub const MAX_DIAGNOSTIC_BASELINE_BYTES: usize = 64 * 1024;
const MAX_DIAGNOSTICS: usize = 128;

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
        let diagnostics = redacted_diagnostics(
            evidence
                .diagnostics()
                .map_err(|_| DiagnosticEvidenceError::Incomplete)?,
        );
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
        let envelope: RawEnvelope =
            serde_json::from_slice(bytes).map_err(|_| DiagnosticBaselineWireError::Malformed)?;
        if envelope.version != DIAGNOSTIC_BASELINE_VERSION {
            return Err(DiagnosticBaselineWireError::Malformed);
        }
        if digest(envelope.payload.get().as_bytes()) != envelope.payload_sha256 {
            return Err(DiagnosticBaselineWireError::DigestMismatch);
        }
        let payload: WirePayload = serde_json::from_str(envelope.payload.get())
            .map_err(|_| DiagnosticBaselineWireError::Malformed)?;
        Self::from_wire(payload)
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

pub(crate) fn redacted_diagnostics(diagnostics: &DiagnosticSet) -> DiagnosticSet {
    let mut entries = diagnostics
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
        .collect::<Vec<_>>();
    entries.sort();
    DiagnosticSet::new(entries)
}

fn digest(bytes: &[u8]) -> Sha256Digest {
    let digest = Sha256::digest(bytes);
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    Sha256Digest::parse(&hex).expect("sha256 encoding is valid")
}
