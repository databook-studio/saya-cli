use super::{CodingMessage, CodingPanel};
use crate::{
    agent::tools::shell_policy::ModeCell,
    cli::CodingCommand,
    commands::coding::{
        CodingApplication, CodingAuthorization, CodingError, CodingJobRequest, Launch,
        dispatch_coding,
    },
    config::runtime::RuntimeConfig,
};
use async_trait::async_trait;
use saya_agent::{ApprovalDecider, CancellationToken, NoopEventSink, ToolDefinition};
use saya_store::SqliteStateStore;
use serde_json::Value;
use std::{
    path::PathBuf,
    sync::{Arc, OnceLock, mpsc},
};
#[path = "worker/active.rs"]
mod active;
#[path = "worker/chat_operator.rs"]
mod chat_operator;
#[cfg(test)]
#[path = "worker/chat_operator_refusal_tests.rs"]
mod chat_operator_refusal_tests;
#[cfg(test)]
#[path = "worker/chat_operator_tests.rs"]
mod chat_operator_tests;
#[cfg(test)]
#[path = "worker/chat_prompt_fixture.rs"]
mod chat_prompt_fixture;
#[path = "worker/wait_policy.rs"]
mod wait_policy;
#[cfg(test)]
mod wait_policy_tests;
#[cfg(test)]
mod worker_tests;
use self::wait_policy::{WaitPolicy, ask_operator};
struct PanelApproval {
    sender: mpsc::SyncSender<CodingMessage>,
    token: CancellationToken,
    id: String,
}
#[async_trait]
impl ApprovalDecider for PanelApproval {
    async fn approve(&self, tool: &ToolDefinition, arguments: &Value) -> bool {
        // The panel answers for exactly one job: a proposal that names any
        // other job (or none) is never put to the operator as this job's.
        if tool.name != "coding_job"
            || arguments.get("job_id").and_then(Value::as_str) != Some(&self.id)
        {
            return false;
        }
        let text = format!(
            "Approve job {} with this exact scope:\n{}\nUse `/job approve {}` or `/job deny {}`.",
            self.id, arguments, self.id, self.id
        );
        ask_operator(&self.sender, &self.token, text, WaitPolicy::PANEL).await
    }
}
/// The root mode a panel launch opens over: the command's flag, its configured
/// default, or — for a lifecycle verb — the job's own record.
fn root_mode(
    command: &CodingCommand,
    runtime: &RuntimeConfig,
) -> Result<crate::cli::RootMode, CodingError> {
    let root = crate::state_path::state_db_path()
        .parent()
        .ok_or(CodingError::StateDirectory)?
        .join("coding");
    Ok(command.root_mode_for(&root, runtime.resolved.coding.root_mode))
}

/// Whether this launch may act without asking the panel: an explicit
/// `--approve`, or an edit-capable consent that already implies it.
pub(super) fn panel_consent(approve: bool, policy: saya_agent::ApprovalPolicy) -> bool {
    approve || crate::commands::coding::consent_implied(policy)
}

pub(super) fn spawn(
    command: CodingCommand,
    runtime: RuntimeConfig,
    store: SqliteStateStore,
    repository: PathBuf,
    approval_policy: saya_agent::ApprovalPolicy,
) -> CodingPanel {
    let (tx, rx) = mpsc::sync_channel(128);
    let (receipt_tx, receipts) = mpsc::sync_channel(64);
    let (done_tx, done) = mpsc::sync_channel(1);
    let (commands, control) = tokio::sync::mpsc::channel(16);
    let cancellation = CancellationToken::new();
    let token = cancellation.clone();
    // The job's one mode cell, published back to the panel as soon as the
    // worker has built the job: the handle the session's own switch states a
    // mode on, so a mode switched in the TUI reaches this job's next call.
    let job_mode = Arc::new(OnceLock::new());
    let published = Arc::clone(&job_mode);
    let id = match &command {
        CodingCommand::Start { id, .. } | CodingCommand::FollowUp { id, .. } => Some(id.clone()),
        _ => None,
    };
    let worker = std::thread::spawn(move || {
        let senders = PanelSenders {
            activity: tx,
            receipts: receipt_tx,
        };
        let result = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| error.to_string())
            .and_then(|runtime_owner| {
                runtime_owner
                    .block_on(work(
                        command,
                        runtime,
                        store,
                        repository,
                        token,
                        senders,
                        control,
                        approval_policy,
                        published,
                    ))
                    .map_err(|error| error.to_string())
            });
        let _ = done_tx.send(result);
    });
    CodingPanel {
        id,
        rx,
        receipts,
        done,
        commands,
        cancellation,
        pending: None,
        worker: Some(worker),
        active: true,
        job_mode,
    }
}
/// Operator-visible outcomes (control receipts, pause acknowledgements) ride
/// their own bounded queue and the send blocks until it has room: a dropped
/// receiver means the panel is gone, so the send is ignored. They must not
/// share the activity queue — a same-tick burst of progress events pushed
/// behind a receipt scrolls it above the viewport before any frame draws it,
/// and the receipt is never painted.
fn deliver_outcome(receipts: &mpsc::SyncSender<CodingMessage>, text: String) {
    let _ = receipts.send(CodingMessage::Activity(text));
}
struct PanelSenders {
    activity: mpsc::SyncSender<CodingMessage>,
    receipts: mpsc::SyncSender<CodingMessage>,
}
// The panel's whole input, threaded once from `spawn`: the command, the
// composed runtime, the store, the workspace, the run's token, its two
// queues, and the live session's approval mode.
#[allow(clippy::too_many_arguments)]
async fn work(
    command: CodingCommand,
    runtime: RuntimeConfig,
    store: SqliteStateStore,
    repository: PathBuf,
    token: CancellationToken,
    senders: PanelSenders,
    controls: tokio::sync::mpsc::Receiver<CodingCommand>,
    approval_policy: saya_agent::ApprovalPolicy,
    job_mode: Arc<OnceLock<ModeCell>>,
) -> Result<String, CodingError> {
    let PanelSenders {
        activity: tx,
        receipts,
    } = senders;
    // A session start with no `--write` edits the whole workspace, like
    // `saya -p`; an explicit `--write` narrows. The panel still puts this
    // exact scope to the operator.
    let command = command.with_session_write_scope();
    // The job id the panel's own approval answers for, when this command runs
    // a root with the panel open.
    let shell_id = match &command {
        CodingCommand::Start { id, .. } | CodingCommand::FollowUp { id, .. } => Some(id.clone()),
        _ => None,
    };
    let mode = root_mode(&command, &runtime)?;
    // Only a start may take the fallback; every other verb opens over the
    // mode its job records.
    let (application, _) = if matches!(command, CodingCommand::Start { .. }) {
        CodingApplication::session_for_start(runtime, store, &repository, mode, &token).await?
    } else {
        (
            CodingApplication::session(runtime, store, &repository, mode, &token).await?,
            None,
        )
    };
    // The live session's mode, stated once at composition: the root's `shell`
    // reads it to decide whether a run outside the sandbox is put to the
    // operator. The panel is an attached surface, so its mode stays
    // switchable.
    let application = Launch::Attached(approval_policy).compose(application);
    // A job started from this panel asks its egressing calls in the chat's
    // approval prompt, waiting on the job's own token for the person sitting
    // at the TUI. Any other command runs no root, so it keeps no operator.
    let application = if shell_id.is_some() {
        application.with_operator(Arc::new(chat_operator::ChatOperator::new(
            tx.clone(),
            token.clone(),
        )))
    } else {
        application
    };
    if matches!(command, CodingCommand::FollowUp { .. }) {
        let (job, request) = resume_follow_up(&application, command, token, tx.clone()).await?;
        let _ = job_mode.set(job.approval_mode());
        let run = async {
            job.run_follow_up(request)
                .await
                .map(|value| value.to_string())
        };
        return active::drive(
            &job,
            run,
            PanelSenders {
                activity: tx,
                receipts,
            },
            controls,
        )
        .await;
    }
    let CodingCommand::Start {
        id,
        task,
        write_paths,
        database_profiles,
        tools,
        plan,
        approve,
        in_place: _,
        managed: _,
    } = command
    else {
        let shown = command.clone();
        return dispatch_coding(&application, command)
            .await
            .map(|value| crate::render::render_coding_result(&shown, &value));
    };
    // The scope already carries the session default: the workspace root when
    // no `--write` was stated, the stated paths otherwise.
    let request = CodingJobRequest::new(
        saya_types::JobId::parse(&id).map_err(CodingError::owner)?,
        task,
        write_paths,
    )?
    .with_database_profiles(database_profiles)?
    .with_tools_allow(application.stated_tools(&tools)?)?
    .with_plan(plan)?;
    let (application, proposed, plan, fallback) =
        application.plan_for_start(&request, &token).await?;
    if let Some(reason) = fallback {
        deliver_outcome(
            &receipts,
            crate::commands::coding::root_mode::notice(reason),
        );
    }
    let approval: Box<dyn ApprovalDecider> = if panel_consent(approve, approval_policy) {
        Box::new(CodingAuthorization::scoped(&proposed)?)
    } else {
        Box::new(PanelApproval {
            sender: tx.clone(),
            token: token.clone(),
            id,
        })
    };
    let job = application
        .create_for_start(request, plan, approval.as_ref(), token)
        .await?;
    let _ = job_mode.set(job.approval_mode());
    let run = async {
        job.run(&NoopEventSink).await.map(|output| {
            // The model's final answer, not the readiness envelope the
            // internal output carries for a write-path job.
            let answer = job
                .outcome()
                .and_then(|outcome| outcome["answer"].as_str().map(str::to_owned))
                .unwrap_or(output.answer);
            match job.failure() {
                Some(failure) => {
                    format!("{answer}\nJob stopped: {}", failure.class.as_str())
                }
                None => answer,
            }
        })
    };
    active::drive(
        &job,
        run,
        PanelSenders {
            activity: tx,
            receipts,
        },
        controls,
    )
    .await
}

async fn resume_follow_up(
    application: &Arc<CodingApplication>,
    command: CodingCommand,
    token: CancellationToken,
    tx: mpsc::SyncSender<CodingMessage>,
) -> Result<
    (
        Arc<crate::commands::coding::NativeCodingJob>,
        crate::commands::coding::ChildFollowUpRequest,
    ),
    CodingError,
> {
    let CodingCommand::FollowUp {
        id,
        agent,
        expected_completion_sequence,
        instruction,
        approve,
        in_place: _,
        managed: _,
    } = command
    else {
        return Err(CodingError::InvalidRequest);
    };
    let job_id = saya_types::JobId::parse(&id).map_err(CodingError::owner)?;
    let approval: Box<dyn ApprovalDecider> = if approve {
        Box::new(CodingAuthorization::scoped(
            &application.propose_resume(&job_id).await?,
        )?)
    } else {
        Box::new(PanelApproval {
            sender: tx,
            token: token.clone(),
            id,
        })
    };
    let job = application
        .resume(&job_id, approval.as_ref(), token)
        .await?;
    let request = crate::commands::coding::ChildFollowUpRequest {
        agent_id: saya_types::AgentId::parse(&agent).map_err(CodingError::owner)?,
        expected_completion_sequence: std::num::NonZeroU64::new(expected_completion_sequence)
            .ok_or(CodingError::InvalidRequest)?,
        instruction,
    };
    Ok((job, request))
}

#[cfg(all(test, unix))]
#[path = "worker/follow_up_tests.rs"]
mod follow_up_tests;
