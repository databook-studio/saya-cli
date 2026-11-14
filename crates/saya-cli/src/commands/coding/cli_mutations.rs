use super::{CodingApplication, CodingAuthorization, CodingError};
use crate::cli::CodingCommand;
use saya_agent::CancellationToken;
use saya_types::{JobId, ManagedCheckoutId, Sha256Digest};
use serde_json::{Value, json};
use std::sync::Arc;
pub(super) async fn run(
    app: &Arc<CodingApplication>,
    command: CodingCommand,
) -> Result<Value, CodingError> {
    let (id, approved) = match &command {
        CodingCommand::FollowUp { id, approve, .. }
        | CodingCommand::Review { id, approve, .. }
        | CodingCommand::ReviewExisting { id, approve, .. }
        | CodingCommand::Apply { id, approve, .. }
        | CodingCommand::RemoveCheckout { id, approve, .. }
        | CodingCommand::ReconcileCheckout { id, approve, .. }
        | CodingCommand::RetainCheckout { id, approve, .. } => (id, *approve),
        _ => return Err(CodingError::InvalidRequest),
    };
    if !approved {
        return Err(CodingError::ApprovalRequired);
    }
    let id = JobId::parse(id).map_err(CodingError::owner)?;
    let consent = CodingAuthorization::scoped(&app.propose_resume(&id).await?)?;
    // A lifecycle verb acts on a job that is not running, so it never takes
    // the one-live-job lease over an in-place root.
    let job = app
        .resume_lifecycle(&id, &consent, CancellationToken::new())
        .await?;
    match command {
        CodingCommand::FollowUp {
            agent,
            expected_completion_sequence,
            instruction,
            ..
        } => {
            let request = super::ChildFollowUpRequest {
                agent_id: saya_types::AgentId::parse(&agent).map_err(CodingError::owner)?,
                expected_completion_sequence: std::num::NonZeroU64::new(
                    expected_completion_sequence,
                )
                .ok_or(CodingError::InvalidRequest)?,
                instruction,
            };
            job.run_follow_up(request).await
        }
        CodingCommand::Review { candidate, .. } => {
            let candidate = ManagedCheckoutId::parse(&candidate).map_err(CodingError::owner)?;
            Ok(job.review(&candidate).await?.preview().clone())
        }
        CodingCommand::ReviewExisting { candidate, .. } => {
            let candidate = ManagedCheckoutId::parse(&candidate).map_err(CodingError::owner)?;
            Ok(job.review_existing(&candidate).await?.preview().clone())
        }
        CodingCommand::Apply {
            candidate,
            source_sha256,
            ..
        } => {
            let candidate = ManagedCheckoutId::parse(&candidate).map_err(CodingError::owner)?;
            let expected = Sha256Digest::parse(&source_sha256).map_err(CodingError::owner)?;
            let review = job.review_existing(&candidate).await?;
            if review.source_sha256() != &expected {
                return Err(CodingError::StaleReview);
            }
            job.apply_review(&review, true).await
        }
        CodingCommand::RemoveCheckout { checkout, .. } => {
            job.remove_checkout(&ManagedCheckoutId::parse(&checkout).map_err(CodingError::owner)?)
                .await?;
            Ok(json!({"removed":checkout}))
        }
        CodingCommand::ReconcileCheckout { checkout, .. } => {
            job.reconcile_checkout(
                &ManagedCheckoutId::parse(&checkout).map_err(CodingError::owner)?,
            )
            .await
        }
        CodingCommand::RetainCheckout { checkout, .. } => {
            job.retain_checkout(&ManagedCheckoutId::parse(&checkout).map_err(CodingError::owner)?)
                .await?;
            Ok(json!({"retained":checkout}))
        }
        _ => Err(CodingError::InvalidRequest),
    }
}
