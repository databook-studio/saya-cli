use super::{
    CodingApplication, CodingAuthorization, CodingError, CodingJobRequest, host::private_directory,
};
use crate::{cli::CodingCommand, config::runtime::RuntimeConfig, render::RenderFormat};
use saya_agent::{CancellationToken, NoopEventSink};
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
        command @ (CodingCommand::Review { .. }
        | CodingCommand::ReviewExisting { .. }
        | CodingCommand::Apply { .. }
        | CodingCommand::ResolveConflict { .. }
        | CodingCommand::RemoveCheckout { .. }
        | CodingCommand::ReconcileCheckout { .. }
        | CodingCommand::RetainCheckout { .. }) => {
            super::cli_mutations::run(application, command).await
        }
        CodingCommand::Resume { id, approve } => {
            if !approve {
                return Err(CodingError::ApprovalRequired);
            }
            let id = JobId::parse(&id).map_err(CodingError::owner)?;

            let consent = CodingAuthorization::scoped(&application.propose_resume(&id).await?)?;
            let job = application
                .resume(&id, &consent, CancellationToken::new())
                .await?;
            let output = job.run(&NoopEventSink).await?;
            Ok(
                json!({"job_id":id,"answer":saya_types::redact(&output.answer),"truncated":output.truncated}),
            )
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
            programs,
            approve,
        } => {
            if !approve {
                return Err(CodingError::ApprovalRequired);
            }
            let request = CodingJobRequest::new(
                JobId::parse(&id).map_err(CodingError::owner)?,
                task,
                write_paths,
                programs,
            )?;
            let consent = CodingAuthorization::scoped(&application.propose(&request).await?)?;
            let job = application
                .create(request, &consent, CancellationToken::new())
                .await?;
            let output = job.run(&NoopEventSink).await?;
            Ok(
                json!({"job_id":job.job_id(),"answer":saya_types::redact(&output.answer),"truncated":output.truncated}),
            )
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
        CodingCommand::Worktrees { id } => {
            application
                .worktrees(&JobId::parse(&id).map_err(CodingError::owner)?)
                .await
        }
        CodingCommand::Conflicts { id, candidate } => {
            application
                .conflicts(
                    &JobId::parse(&id).map_err(CodingError::owner)?,
                    &saya_types::ManagedCheckoutId::parse(&candidate)
                        .map_err(CodingError::owner)?,
                )
                .await
        }
    }
}

pub(crate) async fn run(
    command: CodingCommand,
    repository: PathBuf,
    program_dir: PathBuf,
    runtime: &RuntimeConfig,
    format: RenderFormat,
    store: &SqliteStateStore,
) -> Result<i32, Box<dyn std::error::Error>> {
    let result = async {
        let database = crate::state_path::state_db_path();
        let root = database
            .parent()
            .ok_or(CodingError::StateDirectory)?
            .join("coding");
        private_directory(&root)?;
        let git = find_git()?;
        let app = CodingApplication::open_default(
            runtime.clone(),
            store.clone(),
            &repository,
            &root,
            &git,
            &program_dir,
            &CancellationToken::new(),
        )
        .await?;
        dispatch(&app, command).await
    }
    .await;
    match result {
        Ok(value) => crate::commands::output::result(serde_json::to_string(&value)?, format),
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
