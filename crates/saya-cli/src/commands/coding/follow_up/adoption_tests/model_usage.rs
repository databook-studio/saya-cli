use saya_agent::{
    EffectAdmission, EffectCallKind, EffectOutcome, EffectSettlement, PhysicalAttempt,
};
use saya_harness::coding::budget::RootBudgetView;

/// A root model request Saya gave up on: its one send and the request itself
/// settle with unknown usage, before the root's later tool effect.
pub(super) async fn give_up_request(view: &RootBudgetView) {
    let model = view.admit_model_request().await.unwrap();
    let attempt = PhysicalAttempt::new(model, EffectCallKind::Answer, 0);
    let send = view.admit_physical_attempt(attempt).await.unwrap();
    view.settle(
        send,
        EffectSettlement::PhysicalAttempt {
            attempt,
            outcome: EffectOutcome::Unknown,
            reported_usage: None,
            usage_complete: false,
        },
    )
    .await
    .unwrap();
    view.settle(
        model,
        EffectSettlement::ModelRequest {
            outcome: EffectOutcome::Unknown,
            reported_usage: None,
            usage_complete: false,
        },
    )
    .await
    .unwrap();
}
