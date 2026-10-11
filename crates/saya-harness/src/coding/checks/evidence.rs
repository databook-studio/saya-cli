//! Receipt-bound diagnostics. No public constructor accepts arbitrary tool bytes.
use std::path::Path;

use saya_agent::CancellationToken;
use saya_types::{CheckDiagnosticFormat, CodingCheckOutcome};

use super::{
    CheckExecutionError, CheckReceipt, DiagnosticDelta, DiagnosticError, DiagnosticSet,
    PersistedDiagnosticBaseline, diagnostics,
};
use crate::coding::{
    commands::{CommandOutputCapture, CommandOutputError, OutputStream},
    worktree::OwnershipInput,
};

#[derive(Debug, Clone, thiserror::Error)]
#[non_exhaustive]
pub enum CheckDiagnosticError {
    #[error("complete diagnostic output was unavailable")]
    Capture(#[from] CommandOutputError),
    #[error("diagnostic output could not be parsed")]
    Parse(#[from] DiagnosticError),
    #[error("the check did not have a known normal process exit")]
    ProcessUncertain,
    #[error("diagnostic capture does not belong to the selected check stream")]
    OriginMismatch,
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DiagnosticEvidenceError {
    #[error("check evidence could not be renewed")]
    Execution(#[from] CheckExecutionError),
    #[error("complete diagnostic evidence is unavailable")]
    Incomplete,
    #[error("the diagnostic checks are not comparable")]
    Incomparable,
}

/// Always retains the real joined receipt, including when diagnostics failed.
#[derive(Debug)]
pub struct CheckDiagnostics {
    receipt: CheckReceipt,
    format: CheckDiagnosticFormat,
    diagnostics: Result<DiagnosticSet, CheckDiagnosticError>,
}

/// A borrowed, explicit baseline minted while its receipt was fresh. It grants
/// neither process success nor continuing authority over a mutable checkout.
#[derive(Debug)]
pub struct DiagnosticBaseline<'a> {
    evidence: &'a CheckDiagnostics,
}

pub(super) fn diagnostic_stream(
    format: CheckDiagnosticFormat,
) -> Result<OutputStream, CheckExecutionError> {
    match format {
        CheckDiagnosticFormat::RustcJson => Ok(OutputStream::Stderr),
        CheckDiagnosticFormat::CargoJson => Ok(OutputStream::Stdout),
        _ => Err(CheckExecutionError::DiagnosticFormatRequired),
    }
}

impl CheckDiagnostics {
    pub(super) fn from_run(
        receipt: CheckReceipt,
        format: CheckDiagnosticFormat,
        capture: Option<Result<CommandOutputCapture, CommandOutputError>>,
        root: &Path,
    ) -> Self {
        let diagnostics = capture
            .ok_or(CheckDiagnosticError::OriginMismatch)
            .and_then(|capture| capture.map_err(CheckDiagnosticError::Capture))
            .and_then(|capture| {
                if diagnostic_stream(format).ok() != Some(capture.stream())
                    || capture.directory_id() != receipt.output().directory_id()
                {
                    return Err(CheckDiagnosticError::OriginMismatch);
                }
                if !matches!(receipt.outcome(), CodingCheckOutcome::KnownExit { .. }) {
                    return Err(CheckDiagnosticError::ProcessUncertain);
                }
                diagnostics::parse(format, capture.as_bytes(), root)
                    .map_err(CheckDiagnosticError::Parse)
            });
        Self {
            receipt,
            format,
            diagnostics,
        }
    }

    pub fn receipt(&self) -> &CheckReceipt {
        &self.receipt
    }

    pub fn format(&self) -> CheckDiagnosticFormat {
        self.format
    }

    pub fn diagnostics(&self) -> Result<&DiagnosticSet, &CheckDiagnosticError> {
        self.diagnostics.as_ref()
    }

    pub async fn baseline<'a>(
        &'a self,
        ownership: &OwnershipInput<'_>,
        token: &CancellationToken,
    ) -> Result<DiagnosticBaseline<'a>, DiagnosticEvidenceError> {
        self.receipt.verify_at_use(ownership, token).await?;
        self.complete()?;
        Ok(DiagnosticBaseline { evidence: self })
    }

    pub async fn compare_baseline(
        &self,
        baseline: &DiagnosticBaseline<'_>,
        ownership: &OwnershipInput<'_>,
        token: &CancellationToken,
    ) -> Result<DiagnosticDelta, DiagnosticEvidenceError> {
        self.receipt.verify_at_use(ownership, token).await?;
        let current = self.complete()?;
        let old = baseline.evidence;
        let previous = old.complete()?;
        if self.format != old.format
            || current.parser_revision() != previous.parser_revision()
            || !self.receipt.comparable(&old.receipt)
        {
            return Err(DiagnosticEvidenceError::Incomparable);
        }
        // The old receipt was renewed at mint. Rechecking it against the edited
        // tree here would reject the very source change this comparison measures.
        Ok(DiagnosticDelta::between(
            previous,
            current,
            &old.receipt,
            &self.receipt,
        ))
    }

    pub fn persisted_baseline(
        &self,
        baseline: &DiagnosticBaseline<'_>,
    ) -> Result<PersistedDiagnosticBaseline, DiagnosticEvidenceError> {
        PersistedDiagnosticBaseline::from_minted(self, baseline)
    }

    pub async fn compare_persisted_baseline(
        &self,
        baseline: &PersistedDiagnosticBaseline,
        ownership: &OwnershipInput<'_>,
        token: &CancellationToken,
    ) -> Result<DiagnosticDelta, DiagnosticEvidenceError> {
        self.receipt.verify_at_use(ownership, token).await?;
        let current = self.complete()?;
        if self.format != baseline.format
            || current.parser_revision() != baseline.parser_revision
            || !super::diagnostics::baseline_wire::receipt_matches(&self.receipt, &baseline.key)
        {
            return Err(DiagnosticEvidenceError::Incomparable);
        }
        Ok(DiagnosticDelta::between_persisted(
            &baseline.diagnostics,
            current,
            baseline.key.source_sha256.clone(),
            baseline.key.outcome,
            &self.receipt,
        ))
    }

    fn complete(&self) -> Result<&DiagnosticSet, DiagnosticEvidenceError> {
        if !self.receipt.output().complete()
            || !matches!(self.receipt.outcome(), CodingCheckOutcome::KnownExit { .. })
        {
            return Err(DiagnosticEvidenceError::Incomplete);
        }
        self.diagnostics
            .as_ref()
            .map_err(|_| DiagnosticEvidenceError::Incomplete)
    }
}
