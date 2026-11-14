use super::{
    CodingApplication, CodingAuthorization, CodingError, CodingJobRequest, host::private_directory,
};
use crate::{cli::CodingCommand, config::runtime::RuntimeConfig, render::RenderFormat};
use saya_agent::{ApprovalPolicy, CancellationToken, NoopEventSink};
use saya_store::SqliteStateStore;
use saya_types::JobId;
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Arc};

pub async fn dispatch(
    application: &Arc<CodingApplication>,
    command: CodingCommand,
) -> Result<Value, CodingError> {
    match command {
        CodingCommand::Export {
            id,
            after,
            transport_after,
        } => {
            application
                .export(
                    &JobId::parse(&id).map_err(CodingError::owner)?,
                    after
                        .map(|value| {
                            std::num::NonZeroU64::new(value).ok_or(CodingError::InvalidRequest)
                        })
                        .transpose()?,
                    transport_after
                        .map(|value| {
                            std::num::NonZeroU64::new(value).ok_or(CodingError::InvalidRequest)
                        })
                        .transpose()?,
                )
                .await
        }
        CodingCommand::Events { id, after } => {
            let job = JobId::parse(&id).map_err(CodingError::owner)?;
            application.status(&job).await?;
            let after = after
                .map(|value| std::num::NonZeroU64::new(value).ok_or(CodingError::InvalidRequest))
                .transpose()?;
            let page = application
                .store
                .list_coding_events(&job, after, 100)
                .await
                .map_err(CodingError::owner)?;
            Ok(json!({"events":page.events,"next_cursor":page.next_cursor}))
        }
        command @ (CodingCommand::FollowUp { .. }
        | CodingCommand::Review { .. }
        | CodingCommand::ReviewExisting { .. }
        | CodingCommand::Apply { .. }
        | CodingCommand::RemoveCheckout { .. }
        | CodingCommand::ReconcileCheckout { .. }
        | CodingCommand::RetainCheckout { .. }) => {
            super::cli_mutations::run(application, command).await
        }
        CodingCommand::Resume { id, approve } => {
            if !approve && !super::consent_implied(application.approval_policy) {
                return Err(CodingError::ApprovalRequired);
            }
            let id = JobId::parse(&id).map_err(CodingError::owner)?;

            let consent = CodingAuthorization::scoped(&application.propose_resume(&id).await?)?;
            let job = application
                .resume(&id, &consent, CancellationToken::new())
                .await?;
            let started = std::time::Instant::now();
            let run = job.run(&NoopEventSink).await;
            let result = super::oneshot_finish(application, &job, run, started.elapsed()).await;
            Ok(result.value)
        }
        CodingCommand::Notes { id } => {
            application
                .notes(&JobId::parse(&id).map_err(CodingError::owner)?)
                .await
        }
        CodingCommand::Accept { id, sequence } => {
            let job = JobId::parse(&id).map_err(CodingError::owner)?;
            application.status(&job).await?;
            let sequence =
                std::num::NonZeroU64::new(sequence).ok_or(CodingError::InvalidRequest)?;
            let accepted = saya_harness::coding::coordinator::acknowledge_completion(
                &application.store,
                &job,
                sequence,
            )
            .await
            .map_err(CodingError::owner)?;
            Ok(json!({"accepted":accepted,"authority":"none"}))
        }
        CodingCommand::Promote {
            id,
            sequence,
            text,
            approve,
        } => {
            application
                .promote_finding(
                    &JobId::parse(&id).map_err(CodingError::owner)?,
                    std::num::NonZeroU64::new(sequence).ok_or(CodingError::InvalidRequest)?,
                    &text,
                    approve,
                )
                .await
        }

        CodingCommand::Approve { .. }
        | CodingCommand::Deny { .. }
        | CodingCommand::Message { .. }
        | CodingCommand::Pause { .. }
        | CodingCommand::Continue { .. }
        | CodingCommand::CancelChild { .. } => Err(CodingError::NotOwned),
        CodingCommand::Start {
            id,
            task,
            write_paths,
            database_profiles,
            tools,
            plan,
            approve,
            in_place: _,
            managed: _,
        } => {
            if !approve && !super::consent_implied(application.approval_policy) {
                return Err(CodingError::ApprovalRequired);
            }
            // An edit-capable consent answers the approve question and gives
            // an empty scope the folder; `ask` keeps both as stated.
            let write_paths = super::launch_write_scope(application.approval_policy, &write_paths);
            let request = CodingJobRequest::new(
                JobId::parse(&id).map_err(CodingError::owner)?,
                task,
                write_paths,
            )?
            .with_database_profiles(database_profiles)?
            .with_tools_allow(application.stated_tools(&tools)?)?
            .with_plan(plan)?;
            let token = CancellationToken::new();
            let (application, proposed, plan, fallback) =
                application.plan_for_start(&request, &token).await?;
            let consent = CodingAuthorization::scoped(&proposed)?;
            let job = application
                .create_for_start(request, plan, &consent, token)
                .await?;
            if let Some(reason) = fallback {
                super::root_mode::write_fallback(&job.records, reason)?;
            }
            job.run(&NoopEventSink).await?;
            let progress = super::progress_report::read(&job.records);
            let usage = super::usage::read(&application, job.job_id()).await?;
            let mut result = json!({"job_id":job.job_id(),"root_mode":application.root_mode_name(),"outcome":job.outcome(),"failure":job.failure(),"progress":progress,"usage":usage});
            if let Some(reason) = fallback {
                result["managed_fallback"] = json!({"reason": reason});
            }
            Ok(result)
        }
        CodingCommand::Show { id } => {
            application
                .status(&JobId::parse(&id).map_err(CodingError::owner)?)
                .await
        }
        CodingCommand::Cancel { id } => {
            application
                .cancel(&JobId::parse(&id).map_err(CodingError::owner)?)
                .await
        }
        CodingCommand::List { after } => {
            let after = after
                .as_deref()
                .map(JobId::parse)
                .transpose()
                .map_err(CodingError::owner)?;
            application.list(after.as_ref()).await
        }
        CodingCommand::Doctor { probe } => application.doctor(probe).await,
        CodingCommand::Setup { probe } => application.setup(probe).await,
        CodingCommand::Reconcile {
            id,
            plan_sha256,
            approve,
        } => {
            let id = JobId::parse(&id).map_err(CodingError::owner)?;
            match (plan_sha256, approve) {
                (None, false) => application.reconcile_plan(&id).await,
                (Some(digest), true) => {
                    let digest =
                        saya_types::Sha256Digest::parse(&digest).map_err(CodingError::owner)?;
                    application.reconcile(&id, &digest).await
                }
                (Some(_), false) => Err(CodingError::ApprovalRequired),
                (None, true) => Err(CodingError::InvalidRequest),
            }
        }
        CodingCommand::Worktrees { id } => {
            application
                .worktrees(&JobId::parse(&id).map_err(CodingError::owner)?)
                .await
        }
        CodingCommand::Rewind {
            id,
            to,
            turn,
            baseline,
            force,
        } => {
            let job = JobId::parse(&id).map_err(CodingError::owner)?;
            if baseline {
                if to.is_some() || turn.is_some() {
                    return Err(CodingError::InvalidRequest);
                }
                application.rewind_baseline(&job, force).await
            } else {
                let to = super::rewind::target(to, turn)?;
                application.rewind(&job, to, force).await
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run(
    command: CodingCommand,
    repository: PathBuf,
    program_dir: PathBuf,
    runtime: &RuntimeConfig,
    format: RenderFormat,
    store: &SqliteStateStore,
    approval_policy: ApprovalPolicy,
) -> Result<i32, Box<dyn std::error::Error>> {
    // A resume answers with the one-shot result document and the process code
    // it states; every other verb keeps its own result envelope.
    let resume = matches!(command, CodingCommand::Resume { .. });
    let result = async {
        let database = crate::state_path::state_db_path();
        let root = database
            .parent()
            .ok_or(CodingError::StateDirectory)?
            .join("coding");
        private_directory(&root)?;
        let mode = command.root_mode_for(&root, runtime.resolved.coding.root_mode);
        let git = find_git()?;
        // Only a start may take the fallback: a lifecycle verb opens over the
        // mode its job records.
        let app = if matches!(command, CodingCommand::Start { .. }) {
            CodingApplication::open_for_start(
                runtime.clone(),
                store.clone(),
                &repository,
                &root,
                &git,
                &program_dir,
                mode,
                &CancellationToken::new(),
            )
            .await?
            .0
        } else {
            CodingApplication::open_default_with_mode(
                runtime.clone(),
                store.clone(),
                &repository,
                &root,
                &git,
                &program_dir,
                mode,
                &CancellationToken::new(),
            )
            .await?
        }
        .with_approval_policy(approval_policy);
        dispatch(&app, command).await
    }
    .await;
    match result {
        Ok(mut value) if resume => {
            let code = value
                .get("exit_code")
                .and_then(Value::as_i64)
                .and_then(|code| i32::try_from(code).ok())
                .unwrap_or(0);
            // The terminal event supplies the `job_result` tag; the library
            // value keeps its own `event` key for the `saya start` envelope.
            if let Some(object) = value.as_object_mut() {
                object.remove("event");
            }
            crate::commands::output::emit(
                crate::render::TerminalEvent::JobResult { value },
                format,
            );
            Ok(code)
        }
        Ok(value) => {
            if let Some(reason) = value
                .get("managed_fallback")
                .and_then(|fallback| fallback.get("reason"))
                .and_then(Value::as_str)
            {
                crate::commands::output::emit(
                    crate::render::TerminalEvent::Diagnostic {
                        message: super::root_mode::notice(reason),
                    },
                    format,
                );
            }
            crate::commands::output::result(serde_json::to_string(&value)?, format)
        }
        Err(error) => crate::commands::output::failure_message(5, error.to_string(), format),
    }
}
pub(super) fn find_git() -> Result<PathBuf, CodingError> {
    std::env::var_os("PATH")
        .and_then(|path| {
            std::env::split_paths(&path)
                .map(|directory| directory.join("git"))
                .find(|candidate| candidate.is_file())
        })
        .ok_or(CodingError::InvalidRequest)
        .and_then(|path| path.canonicalize().map_err(CodingError::owner))
}
