//! The durable audit of job-level approvals. Recording is one-way: consent
//! flows write the audit, the audit never feeds consent. A recorded request is
//! never a grant and no decision here branches on a stored row; the human (or
//! the scoped one-shot authorization) answered before anything is closed.

use super::{CodingApplication, CodingError, create::digest};
use saya_harness::coding::coordinator::inspect_recovery;
use saya_store::StoreError;
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

/// A recorded request waiting for its closure. The closure's own sequence is
/// allocated by that append, so nothing here holds a number read earlier.
pub(super) struct AuditedRequest {
    request_sequence: NonZeroU64,
    expires_at_unix_ms: u64,
}

impl AuditedRequest {
    pub(super) fn is_expired(&self) -> bool {
        now_ms() >= self.expires_at_unix_ms
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        })
}

impl CodingApplication {
    /// Records the request for a job-level decision. Only a `Running` job can
    /// carry the audit (the store refuses anything else), so a paused job is
    /// not audited and `None` comes back. The request names the exact
    /// proposal by digest, and takes the sequence its append allocates: no
    /// number this verb read before the append can be taken from the live
    /// turn it would have to share the stream with.
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
        let mut request_sequence = None;
        let mut expires_at_unix_ms = 0;
        let appended = self
            .store
            .record_coding_approval_request_at_next_sequence(job, |sequence| {
                let request = ApprovalRequestV1::new_for_coordinator(
                    job.clone(),
                    sequence,
                    ApprovalRequestContext::new(
                        proposal.clone(),
                        epoch,
                        requested,
                        requested.saturating_add(AUDIT_WINDOW_MS),
                    )
                    .map_err(|_| StoreError::Invalid)?,
                    ApprovalScope::job_operation(action, proposal.clone()),
                )
                .map_err(|_| StoreError::Invalid)?;
                expires_at_unix_ms = request.expires_at_unix_ms();
                request_sequence = Some(sequence);
                CodingEventV1::new(
                    sequence,
                    job.clone(),
                    None,
                    None,
                    CodingEventKind::ApprovalRequested {
                        request: Box::new(request),
                    },
                )
                .map_err(|_| StoreError::Invalid)
            })
            .await;
        match appended {
            Ok(_) => {}
            Err(StoreError::Conflict) => return Err(CodingError::Busy),
            Err(error) => return Err(CodingError::owner(error)),
        }
        let request_sequence = request_sequence.ok_or(CodingError::InvalidRequest)?;
        Ok(Some(AuditedRequest {
            request_sequence,
            expires_at_unix_ms,
        }))
    }

    /// Closes a recorded request with the decision that was actually made.
    pub(super) async fn audit_close(
        &self,
        job: &JobId,
        request: AuditedRequest,
        disposition: ApprovalDisposition,
    ) -> Result<(), CodingError> {
        self.close_audit(job, request.request_sequence, disposition)
            .await
    }

    async fn close_audit(
        &self,
        job: &JobId,
        request_sequence: NonZeroU64,
        disposition: ApprovalDisposition,
    ) -> Result<(), CodingError> {
        self.store
            .close_coding_approval_at_next_sequence(job, |sequence| {
                CodingEventV1::new(
                    sequence,
                    job.clone(),
                    None,
                    None,
                    CodingEventKind::ApprovalClosed {
                        request_sequence,
                        disposition: disposition.clone(),
                    },
                )
                .map_err(|_| StoreError::Invalid)
            })
            .await
            .map_err(CodingError::owner)?;
        Ok(())
    }

    /// A process that died while waiting for a decision leaves its request
    /// open, and an open request blocks resume. When that request is the SOLE
    /// blocker (no live child, unresolved effect or checkout operation: a
    /// genuinely live job is never touched) it is closed `Expired` before a
    /// new question is asked.
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
        let now = now_ms();
        let mut cursor = None;
        loop {
            let page = self
                .store
                .list_coding_approval_requests(job, cursor, 100)
                .await
                .map_err(CodingError::owner)?;
            for record in page.items.iter().filter(|record| {
                record.closure.is_none() && record.request.expires_at_unix_ms() <= now
            }) {
                self.close_audit(
                    job,
                    record.request.request_sequence(),
                    ApprovalDisposition::Expired,
                )
                .await?;
            }
            match page.next_cursor {
                Some(after) if Some(after) != cursor => cursor = Some(after),
                _ => return Ok(()),
            }
        }
    }
}
