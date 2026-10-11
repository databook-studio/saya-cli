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
        CodingCommand::Review { id, approve, .. }
        | CodingCommand::Apply { id, approve, .. }
        | CodingCommand::RemoveCheckout { id, approve, .. }
        | CodingCommand::ReconcileCheckout { id, approve, .. } => (id, *approve),
        _ => return Err(CodingError::InvalidRequest),
    };
    if !approved {
        return Err(CodingError::ApprovalRequired);
    }
    let id = JobId::parse(id).map_err(CodingError::owner)?;
    let consent = CodingAuthorization::scoped(&app.propose_resume(&id).await?)?;
    let job = app.resume(&id, &consent, CancellationToken::new()).await?;
    match command {
        CodingCommand::Review { candidate, .. } => {
            let candidate = ManagedCheckoutId::parse(&candidate).map_err(CodingError::owner)?;
            Ok(job.review(&candidate).await?.preview().clone())
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
        _ => Err(CodingError::InvalidRequest),
    }
}
