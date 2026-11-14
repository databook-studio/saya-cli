use std::{path::Path, path::PathBuf, time::Duration};

use crate::{cli::RuntimeCheckAction, render::RenderFormat, state_path};
use saya_agent::CancellationToken;
use saya_harness::{
    coding::git::IgnoredPaths,
    coding::integration::{CandidateError, DEFAULT_DELTA_DEADLINE, operator_project_source_digest},
    fetch::ReqwestTransport,
    runtime::{
        NpmCheckAction, NpmCheckRequest, NpmCheckSource, NpmInstallConsent, install_npm_inputs,
        run_npm_check,
    },
    workspace::Workspace,
};

use super::{completed_operator_result, settle_install, state_root_from_db_path, stop_code_for};

#[path = "npm/render.rs"]
mod render;
use render::{check_node_result, npm_input_message};

#[derive(Debug, thiserror::Error)]
enum CheckNodeError {
    #[error(transparent)]
    Runtime(#[from] saya_harness::runtime::NpmRuntimeError),
    #[error("the project source changed while the check ran")]
    SourceChanged,
    #[error("the project source could not be fingerprinted")]
    Candidate(#[from] CandidateError),
}

pub(crate) fn install_deps(
    project_path: PathBuf,
    format: RenderFormat,
) -> Result<i32, Box<dyn std::error::Error>> {
    let state_root = match state_root_from_db_path(&state_path::state_db_path()) {
        Ok(path) => path,
        Err(message) => return crate::commands::failure_message(3, message, format),
    };
    let project = match Workspace::open(&project_path) {
        Ok(project) => project,
        Err(_) => {
            return crate::commands::failure_message(
                3,
                "the selected project directory could not be opened safely".to_owned(),
                format,
            );
        }
    };
    let transport = ReqwestTransport::new()?;
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(install_deps_cancellable(
            &state_root,
            &project,
            &transport,
            format,
        ))
}

async fn install_deps_cancellable(
    state_root: &Path,
    project: &Workspace,
    transport: &ReqwestTransport,
    format: RenderFormat,
) -> Result<i32, Box<dyn std::error::Error>> {
    let cancellation = CancellationToken::new();
    let work = install_npm_inputs(
        state_root,
        project,
        Some(NpmInstallConsent::Granted),
        transport,
        &cancellation,
    );
    let (outcome, stop) = match settle_install(&cancellation, work).await {
        Ok(settled) => settled,
        Err(error) => {
            return crate::commands::failure_message(
                5,
                format!("could not register runtime stop signals: {error}"),
                format,
            );
        }
    };
    match outcome {
        Ok(receipt) => completed_operator_result(npm_input_message(&receipt), stop, format),
        Err(error) => {
            crate::commands::failure_message(stop_code_for(stop, 5), error.to_string(), format)
        }
    }
}

pub(crate) fn check_node(
    project_path: PathBuf,
    action: RuntimeCheckAction,
    format: RenderFormat,
) -> Result<i32, Box<dyn std::error::Error>> {
    let state_root = match state_root_from_db_path(&state_path::state_db_path()) {
        Ok(path) => path,
        Err(message) => return crate::commands::failure_message(3, message, format),
    };
    let project = match Workspace::open(&project_path) {
        Ok(project) => project,
        Err(_) => {
            return crate::commands::failure_message(
                3,
                "the selected project directory could not be opened safely".to_owned(),
                format,
            );
        }
    };
    let action = match action {
        RuntimeCheckAction::TestCi => NpmCheckAction::TestCi,
        RuntimeCheckAction::Build => NpmCheckAction::Build,
    };
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(check_node_cancellable(
            &state_root,
            &project,
            action,
            format,
        ))
}

async fn check_node_cancellable(
    state_root: &Path,
    project: &Workspace,
    action: NpmCheckAction,
    format: RenderFormat,
) -> Result<i32, Box<dyn std::error::Error>> {
    let cancellation = CancellationToken::new();
    let source = match operator_project_source_digest(
        project.root(),
        &IgnoredPaths::none(),
        &cancellation,
        DEFAULT_DELTA_DEADLINE,
    ) {
        Ok(source) => source,
        Err(error) => {
            return crate::commands::failure_message(3, error.to_string(), format);
        }
    };
    let work = run_npm_check(NpmCheckRequest {
        state_root,
        project,
        source_sha256: source.as_str(),
        source: NpmCheckSource::OperatorProject,
        action,
        timeout: Duration::from_secs(600),
        cancellation: &cancellation,
    });
    let (result, stop) = match settle_install(&cancellation, async {
        let outcome = work.await?;
        if !outcome.joined()
            || outcome.client_pid() == 0
            || outcome.cleanup() == saya_harness::runtime::CleanupStatus::Unknown
            || outcome.action() != action
            || outcome.source_sha256() != source.as_str()
        {
            return Err(CheckNodeError::Runtime(
                saya_harness::runtime::NpmRuntimeError::Unknown,
            ));
        }
        let after = operator_project_source_digest(
            project.root(),
            &IgnoredPaths::none(),
            &cancellation,
            DEFAULT_DELTA_DEADLINE,
        )
        .map_err(CheckNodeError::Candidate)?;
        if after != source {
            return Err(CheckNodeError::SourceChanged);
        }
        Ok::<_, CheckNodeError>(outcome)
    })
    .await
    {
        Ok(settled) => settled,
        Err(error) => {
            return crate::commands::failure_message(
                5,
                format!("could not register runtime stop signals: {error}"),
                format,
            );
        }
    };
    match result {
        Ok(outcome) => check_node_result(&outcome, stop, format),
        Err(CheckNodeError::Runtime(error)) => {
            crate::commands::failure_message(stop_code_for(stop, 5), error.to_string(), format)
        }
        Err(CheckNodeError::SourceChanged) => crate::commands::failure_message(
            stop_code_for(stop, 4),
            "the project source changed while the check ran".to_owned(),
            format,
        ),
        Err(CheckNodeError::Candidate(_)) => crate::commands::failure_message(
            stop_code_for(stop, 3),
            "the project source could not be fingerprinted".to_owned(),
            format,
        ),
    }
}
