use saya_config::{ResolvedAgents, ResolvedJobLimits};
use saya_harness::coding::budget::{
    MAX_COMMAND_SLOTS, MAX_LIVE_PERMITS, MAX_RESERVATIONS, MAX_RESUME_EVENTS,
};
use saya_types::CodingBudgetMode;
use serde_json::{Value, json};

/// The run limits a job is created under: the `[limits.job]` ceilings,
/// beside the two `[agents]` bounds that govern the child lane.
///
/// `None` is the effective unlimited, which is both the uncapped mode and an
/// unstated job-wide ceiling.
pub(super) fn limits(job: &ResolvedJobLimits, agents: &ResolvedAgents) -> Value {
    json!({
        "max_children": agents.max_children,
        "max_concurrent_model_requests": agents.max_concurrent_model_requests,
        "max_logical_model_requests": job.logical_requests().value(),
        "max_physical_transport_attempts": job.transport_attempts().value(),
        "max_tool_calls": job.tool_calls().value(),
        "max_known_tokens": job.known_tokens().value(),
        "max_elapsed_seconds": job.elapsed_seconds().value(),
    })
}

pub(super) fn token_ceilings(job: &ResolvedJobLimits) -> Value {
    let bounded = job.budget_mode == CodingBudgetMode::Bounded;
    let per_loop = job.known_tokens().value();
    let job_total = job.job_tokens().value();
    json!({
        "per_loop_reported_tokens": {
            "limit": per_loop,
            "enforced": bounded && per_loop.is_some(),
            "basis": "provider_reported_usage",
            "note": match (bounded, per_loop) {
                (true, Some(_)) => "each model loop stops once its own reported tokens reach the limit; one in-flight request can overshoot",
                (true, None) => "not enforced unless limits.job.known_tokens is set",
                (false, _) => "uncapped mode removes this configured aggregate token ceiling",
            },
        },
        "job_total": {
            "limit": job_total,
            "enforced": job_total.is_some(),
            "default": "not enforced",
            "note": match (bounded, job_total) {
                (true, Some(_)) => "only provider-reported usage counts; concurrent requests already admitted may settle after the ceiling is reached and overshoot it",
                (true, None) => "not enforced unless limits.job.job_tokens is set",
                (false, _) => "uncapped mode removes this configured aggregate token ceiling",
            },
        },
    })
}

pub(super) fn operational_bounds() -> Value {
    json!({
        "max_reservations_per_job": MAX_RESERVATIONS,
        "max_live_permits": MAX_LIVE_PERMITS,
        "max_command_slots": MAX_COMMAND_SLOTS,
        "max_resume_history_events": MAX_RESUME_EVENTS,
        "note": "uncapped mode removes configured aggregate ceilings; these storage and concurrency bounds remain active",
    })
}
