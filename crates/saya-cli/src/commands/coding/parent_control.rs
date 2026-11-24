//! The control one root turn runs under: the run's limits and approver, the
//! turn's own stop signal, the mailbox the run's first turn carries, and the
//! answering scope a stated turn policy arms.

use super::parent_turn::RootSession;
use super::{CodingError, NativeCodingJob};
use saya_agent::{
    AgentEventSink, AgentRunControl, AnsweringScope, ApprovalDecider, CancellationToken,
    EffectAdmission, ProgressHandle,
};

/// The control a turn runs under, and the answering scope that carries its
/// turn-scoped budget when it states one.
pub(super) struct TurnControl<'a> {
    pub(super) control: AgentRunControl<'a>,
    pub(super) budget: Option<AnsweringScope>,
    /// The elapsed deadline the turn's policy states, for the owner that arms
    /// it — the answering engine leaves that dimension to the caller.
    pub(super) deadline: Option<std::time::Duration>,
}

/// Builds the control of one turn. The limits are the run's, unless a surface
/// stated a turn policy for it — or an active plugin asks for a tighter one,
/// which is read afresh every turn. A stated policy arms a turn-scoped
/// answering scope on its own token, so a budget stop ends this turn and leaves
/// the run running.
pub(super) fn build<'a>(
    job: &'a NativeCodingJob,
    session: &RootSession,
    approver: &'a dyn ApprovalDecider,
    observed: &'a dyn AgentEventSink,
    cancellation: CancellationToken,
    progress: &ProgressHandle,
    root_budget: &'a dyn EffectAdmission,
) -> Result<TurnControl<'a>, CodingError> {
    let config = &job.application.runtime.resolved;
    let policy = job.root_run.turn_policy().or_else(|| {
        session
            .plugins
            .as_ref()
            .and_then(|plugins| plugins.turn_policy())
    });
    let budget = policy.map(|policy| {
        AnsweringScope::new(
            policy.answering_limits(),
            saya_agent::CancellationToken::new(),
        )
    });
    let control = AgentRunControl::new(
        super::super::limits::agent_limits(config),
        super::super::limits::answering_limits(config),
        approver,
        observed,
        cancellation,
    )
    .with_actor_correlation(super::super::create::actor_correlation_digest(&[
        "root",
        job.job_id().as_str(),
    ]))
    .with_physical_effect_admission(root_budget)
    .with_progress_report(progress.clone())
    // A headless run waits one request-deadline slice at a time for a
    // background exit instead of ending and polling; the job's wall clock still
    // bounds the whole wait.
    .with_background_wait(std::time::Duration::from_secs(
        config.ai.request_timeout_seconds,
    ))
    // This job's blocks are the continuation snapshot, not database context, so
    // the framing says only that they are untrusted data.
    .with_context_preamble(saya_agent::ContextPreamble::Coding);
    // The run's mailbox belongs to its first turn: the loop consumes the
    // receiver, so only that turn can carry it.
    let mailbox = session
        .mailbox
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .take();
    let control = match mailbox {
        Some(mailbox) => control.with_mailbox(mailbox),
        None => control,
    };
    let control = control.with_request_preflight(
        crate::agent::preflight_config::request_preflight_config(&config.ai)
            .map_err(|_| CodingError::InvalidRequest)?,
    );
    let control = match budget.as_ref() {
        Some(scope) => control.with_answering_scope(scope.clone()),
        None => control,
    };
    Ok(TurnControl {
        control,
        budget,
        deadline: policy.and_then(|policy| policy.max_elapsed),
    })
}
