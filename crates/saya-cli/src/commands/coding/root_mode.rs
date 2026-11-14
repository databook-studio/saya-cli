//! The root mode a `start` opens over, and the one fallback it may take: a
//! managed-only refusal becomes an in-place start with a stated reason.
//!
//! Both adapters (the CLI launch and the TUI worker) call these functions, so
//! the fallback decision and its notice cannot drift between them.

mod application;
mod classify;

pub(crate) use classify::managed_only_refusal;

use super::{CodingApplication, CodingApprovalRequest, CodingError, CodingJobRequest};
use crate::{cli::RootMode, config::runtime::RuntimeConfig};
use saya_agent::CancellationToken;
use saya_harness::coding::local::JobBaseline;
use saya_store::SqliteStateStore;
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

/// The bound for the private root-mode notice a fallback start writes.
const MAX_ROOT_MODE_RECORD_BYTES: usize = 1024;

/// The private record naming a requested managed start that ran in place.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RootModeRecord {
    requested: String,
    effective: String,
    reason: String,
}

/// The state root and program directory a session opens over. Shared by the
/// session and the start adapters so both resolve the same paths.
pub(super) fn session_inputs(runtime: &RuntimeConfig) -> Result<(PathBuf, PathBuf), CodingError> {
    let database = crate::state_path::state_db_path();
    let root = database
        .parent()
        .ok_or(CodingError::StateDirectory)?
        .join("coding");
    let root = super::host::private_directory(&root)?;
    let program_dir = runtime
        .resolved
        .coding
        .program_dir
        .clone()
        .unwrap_or_else(|| PathBuf::from("/usr/bin"));
    Ok((root, program_dir))
}

/// The one stderr/activity line a fallback start states. Exact text: it is
/// what the user and the session see.
pub(crate) fn notice(reason: &str) -> String {
    format!(
        "managed checkout unavailable ({reason}); working in place — tool edits can be rewound with saya job rewind"
    )
}

/// Writes the fallback notice under the job's records.
pub(crate) fn write_fallback(records: &Path, reason: &str) -> Result<(), CodingError> {
    let record = RootModeRecord {
        requested: "managed".to_owned(),
        effective: "in_place".to_owned(),
        reason: reason.to_owned(),
    };
    let bytes = serde_json::to_vec(&record).map_err(CodingError::owner)?;
    super::context::write_new(
        &records.join("root-mode.json"),
        &bytes,
        MAX_ROOT_MODE_RECORD_BYTES,
    )
}

/// The reason a job's start fell back to in place, when it recorded one.
pub(crate) fn read_fallback(records: &Path) -> Option<String> {
    let bytes =
        super::context::read(&records.join("root-mode.json"), MAX_ROOT_MODE_RECORD_BYTES).ok()?;
    let record: RootModeRecord = serde_json::from_slice(&bytes).ok()?;
    (record.requested == "managed" && record.effective == "in_place").then_some(record.reason)
}

/// Opens the start's application over `requested`, falling back to in place
/// when a managed-only refusal names a class a fallback may absorb.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn open_start(
    runtime: RuntimeConfig,
    store: SqliteStateStore,
    repository: &Path,
    state_root: &Path,
    git: &Path,
    program_dir: &Path,
    requested: RootMode,
    token: &CancellationToken,
) -> Result<(Arc<CodingApplication>, Option<&'static str>), CodingError> {
    match CodingApplication::open_default_with_mode(
        runtime.clone(),
        store.clone(),
        repository,
        state_root,
        git,
        program_dir,
        requested,
        token,
    )
    .await
    {
        Ok(application) => Ok((application, None)),
        Err(error) => {
            let reason = (requested == RootMode::Managed)
                .then(|| managed_only_refusal(&error))
                .flatten();
            // A bare repository cannot work in place either, so it stays the
            // managed refusal it was.
            let Some(reason) = reason.filter(|reason| *reason != "bare_repository") else {
                return Err(error);
            };
            let application = CodingApplication::open_default_with_mode(
                runtime,
                store,
                repository,
                state_root,
                git,
                program_dir,
                RootMode::InPlace,
                token,
            )
            .await?;
            Ok((application.with_managed_fallback(reason), Some(reason)))
        }
    }
}

/// Plans the start over the application, reopening in place once when the
/// managed plan is a refusal a fallback may absorb.
pub(crate) async fn plan_with_fallback(
    application: &Arc<CodingApplication>,
    request: &CodingJobRequest,
    token: &CancellationToken,
) -> Result<
    (
        Arc<CodingApplication>,
        CodingApprovalRequest,
        JobBaseline,
        Option<&'static str>,
    ),
    CodingError,
> {
    match application.propose_with_plan_until(request, token).await {
        Ok((proposal, plan)) => Ok((
            Arc::clone(application),
            proposal,
            plan,
            application.managed_fallback(),
        )),
        Err(error) => {
            // An in-place plan that fails is already in place: reopening it
            // again would mask the real refusal.
            let reason = if application.is_in_place() {
                None
            } else {
                managed_only_refusal(&error)
            };
            let Some(reason) = reason else {
                return Err(error);
            };
            let reopened = application.reopen_in_place(token).await?;
            let reopened = reopened.with_managed_fallback(reason);
            let (proposal, plan) = reopened.propose_with_plan_until(request, token).await?;
            Ok((reopened, proposal, plan, Some(reason)))
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn notice_names_the_reason_and_the_rewind() {
        assert_eq!(
            notice("sparse_checkout"),
            "managed checkout unavailable (sparse_checkout); working in place — tool edits can be rewound with saya job rewind"
        );
    }
}
