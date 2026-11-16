//! The application-side entry points both adapters share.

use super::super::{CodingApplication, CodingApprovalRequest, CodingError, CodingJobRequest};
use super::{open_start, plan_with_fallback, session_inputs};
use crate::cli::RootMode;
use crate::config::runtime::RuntimeConfig;
use saya_agent::CancellationToken;
use saya_harness::coding::local::JobBaseline;
use saya_store::SqliteStateStore;
use std::{path::Path, sync::Arc};

impl CodingApplication {
    /// Opens the start's application: the CLI launch and the TUI worker both
    /// enter here.
    #[allow(clippy::too_many_arguments)]
    pub async fn open_for_start(
        runtime: RuntimeConfig,
        store: SqliteStateStore,
        repository: &Path,
        state_root: &Path,
        git: &Path,
        program_dir: &Path,
        requested: RootMode,
        token: &CancellationToken,
    ) -> Result<(Arc<Self>, Option<&'static str>), CodingError> {
        open_start(
            runtime,
            store,
            repository,
            state_root,
            git,
            program_dir,
            requested,
            token,
        )
        .await
    }

    /// The session adapter's start entry: the same paths a session opens over.
    pub(crate) async fn session_for_start(
        runtime: RuntimeConfig,
        store: SqliteStateStore,
        repository: &Path,
        requested: RootMode,
        token: &CancellationToken,
    ) -> Result<(Arc<Self>, Option<&'static str>), CodingError> {
        let (state_root, program_dir) = session_inputs(&runtime)?;
        let git = super::super::cli::find_git()?;
        open_start(
            runtime,
            store,
            repository,
            &state_root,
            &git,
            &program_dir,
            requested,
            token,
        )
        .await
    }

    /// Plans a start and states the fallback reason, if one applied. Public so
    /// the lifecycle tests can drive the same decision the adapters use.
    pub async fn plan_for_start(
        self: &Arc<Self>,
        request: &CodingJobRequest,
        token: &CancellationToken,
    ) -> Result<
        (
            Arc<Self>,
            CodingApprovalRequest,
            JobBaseline,
            Option<&'static str>,
        ),
        CodingError,
    > {
        plan_with_fallback(self, request, token).await
    }

    /// The fallback reason this application opened with, if any.
    pub fn managed_fallback(&self) -> Option<&'static str> {
        self.managed_fallback
    }

    /// Reopens the same start inputs in place, preserving the operator, the
    /// approval policy, and the personal instruction directory.
    pub(super) async fn reopen_in_place(
        self: &Arc<Self>,
        token: &CancellationToken,
    ) -> Result<Arc<Self>, CodingError> {
        let application = Self::open_with_mode(
            self.runtime.clone(),
            self.store.clone(),
            self.root_path(),
            &self.state_root,
            &self.git,
            &self.program_dir,
            RootMode::InPlace,
            token,
        )
        .await?;
        let application =
            application.with_personal_instruction_dir(self.personal_instruction_dir.as_deref())?;
        let application = application.with_approval_policy(self.approval_policy);
        Ok(match &self.operator {
            Some(operator) => application.with_operator(Arc::clone(operator)),
            None => application,
        })
    }

    /// Records the reason on a fresh application. A shared host keeps the
    /// unset value rather than mutating under another owner.
    pub(super) fn with_managed_fallback(mut self: Arc<Self>, reason: &'static str) -> Arc<Self> {
        if let Some(application) = Arc::get_mut(&mut self) {
            application.managed_fallback = Some(reason);
        }
        self
    }
}
