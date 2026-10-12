//! The durable audit of job-level approvals. Recording is one-way: consent
//! flows write the audit, the audit never feeds consent. A recorded request is
//! never a grant and no decision here branches on a stored row; the human (or
//! the scoped one-shot authorization) answered before anything is closed.

use super::{CodingApplication, CodingError, create::digest};
use saya_harness::coding::coordinator::inspect_recovery;
use saya_types::{
    ApprovalDisposition, ApprovalRequestContext, ApprovalRequestV1, ApprovalScope, CodingEventKind,
    CodingEventV1, JobApprovalAction, JobId, JobState,
};
use serde_json::Value;
use std::{
    num::NonZeroU64,
    time::{SystemTime, UNIX_EPOCH},
};

/// How long a recorded request is a plausible pending decision.
const AUDIT_WINDOW_MS: u64 = 10 * 60 * 1000;

/// A recorded request waiting for its closure; `next` is the next free
/// event sequence of the job's stream.
pub(super) struct AuditedRequest {
    request_sequence: NonZeroU64,
    next: u64,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        })
}

fn sequence(value: u64) -> Result<NonZeroU64, CodingError> {
    NonZeroU64::new(value).ok_or(CodingError::InvalidRequest)
}

impl CodingApplication {
    /// Records the request for a job-level decision. Only a `Running` job can
    /// carry the audit (the store refuses anything else), so a paused job is
    /// not audited and `None` comes back. The request names the exact
    /// proposal by digest.
    pub(super) async fn audit_request(
        &self,
        job: &JobId,
        action: JobApprovalAction,
        proposed: &Value,
        epoch: u64,
    ) -> Result<Option<AuditedRequest>, CodingError> {
        let status = inspect_recovery(&self.store, job)
            .await
            .map_err(CodingError::owner)?;
        if status.state != JobState::Running {
            return Ok(None);
        }
        let proposal = digest(&serde_json::to_vec(proposed).map_err(CodingError::owner)?)?;
        let requested = now_ms();
        let request_sequence = sequence(status.last_sequence + 1)?;
        let request = ApprovalRequestV1::new_for_coordinator(
            job.clone(),
            request_sequence,
            ApprovalRequestContext::new(
                proposal.clone(),
                epoch,
                requested,
                requested.saturating_add(AUDIT_WINDOW_MS),
            )
            .map_err(CodingError::owner)?,
            ApprovalScope::job_operation(action, proposal),
        )
        .map_err(CodingError::owner)?;
        self.store
            .record_coding_approval_request(
                CodingEventV1::new(
                    request_sequence,
                    job.clone(),
                    None,
                    None,
                    CodingEventKind::ApprovalRequested {
                        request: Box::new(request),
                    },
                )
                .map_err(CodingError::owner)?,
            )
            .await
            .map_err(CodingError::owner)?;
        Ok(Some(AuditedRequest {
            request_sequence,
            next: request_sequence.get() + 1,
        }))
    }

    /// Closes a recorded request with the decision that was actually made.
    pub(super) async fn audit_close(
        &self,
        job: &JobId,
        request: AuditedRequest,
        disposition: ApprovalDisposition,
    ) -> Result<(), CodingError> {
        self.close_audit(job, request.request_sequence, request.next, disposition)
            .await
    }

    async fn close_audit(
        &self,
        job: &JobId,
        request_sequence: NonZeroU64,
        at: u64,
        disposition: ApprovalDisposition,
    ) -> Result<(), CodingError> {
        self.store
            .close_coding_approval(
                CodingEventV1::new(
                    sequence(at)?,
                    job.clone(),
                    None,
                    None,
                    CodingEventKind::ApprovalClosed {
                        request_sequence,
                        disposition,
                    },
                )
                .map_err(CodingError::owner)?,
            )
            .await
            .map_err(CodingError::owner)?;
        Ok(())
    }

    /// A process that died while waiting for a decision leaves its request
    /// open, and an open request blocks resume. When that request is the SOLE
    /// blocker (no live child, unresolved effect or checkout operation: a
    /// genuinely live job is never touched) it is closed `Cancelled`, the
    /// truthful disposition for an abandoned wait.
    pub(super) async fn close_stale_approvals(&self, job: &JobId) -> Result<(), CodingError> {
        let status = inspect_recovery(&self.store, job)
            .await
            .map_err(CodingError::owner)?;
        if status.pending_approvals == 0
            || status.active_children != 0
            || status.unresolved_reservations != 0
            || status.unresolved_checkouts != 0
        {
            return Ok(());
        }
        let mut next = status.last_sequence + 1;
        let mut cursor = None;
        loop {
            let page = self
                .store
                .list_coding_approval_requests(job, cursor, 100)
                .await
                .map_err(CodingError::owner)?;
            for record in page.items.iter().filter(|record| record.closure.is_none()) {
                self.close_audit(
                    job,
                    record.request.request_sequence(),
                    next,
                    ApprovalDisposition::Cancelled,
                )
                .await?;
                next += 1;
            }
            match page.next_cursor {
                Some(after) if Some(after) != cursor => cursor = Some(after),
                _ => return Ok(()),
            }
        }
    }
}
