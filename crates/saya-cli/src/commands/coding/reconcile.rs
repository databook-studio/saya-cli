use super::{CodingApplication, CodingError};
use saya_harness::coding::coordinator::{
    ReconcileError, ReconciliationPlan, plan_reconciliation, reconcile_effects,
};
use saya_types::{JobId, Sha256Digest};
use serde_json::{Value, json};

impl CodingApplication {
    /// Lists what blocks a crashed job from resuming. Read-only: it writes
    /// nothing and the returned digest names exactly this list.
    pub async fn reconcile_plan(&self, job: &JobId) -> Result<Value, CodingError> {
        self.status(job).await?;
        let plan = plan_reconciliation(&self.store, job)
            .await
            .map_err(map_error)?;
        Ok(plan_value(&plan))
    }

    /// Records each listed effect as reconciled: its charge stays consumed,
    /// nothing is refunded, killed or re-run, and resume still needs its own
    /// fresh approval. `plan_sha256` must be the digest of the current list.
    pub async fn reconcile(
        &self,
        job: &JobId,
        plan_sha256: &Sha256Digest,
    ) -> Result<Value, CodingError> {
        self.status(job).await?;
        let report = reconcile_effects(&self.store, job, plan_sha256)
            .await
            .map_err(map_error)?;
        Ok(json!({
            "job_id": job,
            "plan_sha256": plan_sha256,
            "reconciled": report.reconciled,
            "refunded": false,
            "next_action": format!("coding resume {job} --approve")
        }))
    }
}

fn plan_value(plan: &ReconciliationPlan) -> Value {
    json!({
        "job_id": plan.job,
        "last_sequence": plan.last_sequence,
        "plan_sha256": plan.digest,
        "effects": plan.effects.iter().map(|effect| json!({
            "reservation_sequence": effect.reservation_sequence,
            "settled_uncertain": effect.settled_uncertain,
            "reserved": effect.reserved
        })).collect::<Vec<_>>(),
        "changes_nothing": true,
        "next_action": if plan.effects.is_empty() {
            "nothing is unresolved; use coding resume".to_owned()
        } else {
            format!(
                "coding reconcile {} --plan-sha256 {} --approve",
                plan.job, plan.digest
            )
        }
    })
}

fn map_error(error: ReconcileError) -> CodingError {
    match error {
        ReconcileError::StalePlan => CodingError::ReconciliationStale,
        refusal @ (ReconcileError::NotReconcilable
        | ReconcileError::ChildrenActive
        | ReconcileError::TooManyEffects) => {
            CodingError::ReconciliationRefused(refusal.to_string())
        }
        other => CodingError::owner(other),
    }
}
