use super::{CodingApplication, CodingError};
use saya_agent::CancellationToken;
use saya_harness::coding::{
    checks::{CheckExecutionInput, CheckInvocation, execute_check, execute_check_with_diagnostics},
    commands::CommandOwner,
    policy::ChildAuthorityView,
    worktree::OwnershipInput,
};
use saya_types::{JobId, ManagedCheckoutId};
use serde_json::{Value, json};
use std::{num::NonZeroU64, path::PathBuf, sync::Arc};

/// One child-bound adapter. Every effect reopens owned checkout evidence; no
/// model argument selects its job, checkout, config, or executable directory.
pub(crate) struct ChildCodingOperations {
    pub(crate) application: Arc<CodingApplication>,
    pub(crate) job: JobId,
    pub(crate) checkout: ManagedCheckoutId,
    pub(crate) authority: ChildAuthorityView,
    pub(crate) owner: Arc<CommandOwner>,
    pub(crate) records: PathBuf,
    pub(crate) failed: std::sync::atomic::AtomicBool,
    pub(crate) sessions: tokio::sync::Mutex<Vec<super::command::ActiveCommand>>,
}
impl ChildCodingOperations {
    pub(crate) fn ownership(&self) -> OwnershipInput<'_> {
        let host = &self.application;
        OwnershipInput {
            store: &host.store,
            service: &host.service,
            repository: &host.repository,
            artifacts: &host.artifacts,
            job_id: &self.job,
            checkout_id: &self.checkout,
        }
    }
    pub(crate) async fn check(
        &self,
        profile: &str,
        token: &CancellationToken,
    ) -> Result<Value, CodingError> {
        let profile = self
            .application
            .runtime
            .resolved
            .coding
            .select_check_profile(profile)
            .map_err(CodingError::owner)?;
        let invocation = invocation(profile)?;
        let ownership = self.ownership();
        let input = CheckExecutionInput {
            ownership: &ownership,
            authority: &self.authority,
            invocation: &invocation,
            owner: &self.owner,
            cancellation: token,
            program_dir: &self.application.program_dir,
            output_root: &self.records,
            resolver: Arc::new(saya_harness::runner::StaticCredentialSource::new([])),
        };
        let mut result = if invocation.diagnostic_format().is_some() {
            let result = execute_check_with_diagnostics(input)
                .await
                .map_err(CodingError::owner)?;
            json!({"receipt":result.receipt().wire_receipt(),"process":process_facts(result.receipt()),"successful":result.receipt().is_successful(),
                "diagnostics_complete":result.diagnostics().is_ok(),"diagnostic_count":result.diagnostics().ok().map(|set|set.entries().len()),"diagnostics":result.diagnostics().ok().map(|set|set.entries().iter().map(|entry|json!({"severity":format!("{:?}",entry.severity()),"code":entry.code(),"message":saya_types::redact(entry.message()),"locations":entry.locations().iter().map(|location|json!({"path":location.path(),"line":location.line_start(),"column":location.column_start()})).collect::<Vec<_>>()})).collect::<Vec<_>>())})
        } else {
            let receipt = execute_check(input).await.map_err(CodingError::owner)?;
            json!({"receipt":receipt.wire_receipt(),"process":process_facts(&receipt),"successful":receipt.is_successful(),"diagnostics_complete":false})
        };
        result["checkout_id"] = json!(self.checkout);
        result["snapshot_id"] = json!(self.authority.snapshot_id());
        result["policy_sha256"] = json!(self.authority.policy_digest());
        result["revocation_epoch"] = json!(self.authority.revocation_epoch());
        super::context::append_line(
            &self
                .records
                .join(format!("checks-{}.jsonl", self.checkout.as_str())),
            &serde_json::to_vec(&result).map_err(CodingError::owner)?,
            1024 * 1024,
        )?;
        Ok(result)
    }
}

pub(super) fn invocation(
    profile: &saya_config::ResolvedCheckProfile,
) -> Result<CheckInvocation, CodingError> {
    let mut invocation = CheckInvocation::new(
        profile.profile_id().into(),
        profile.command_id().into(),
        profile.program().into(),
        profile.argv().to_vec(),
        NonZeroU64::new(profile.timeout_seconds()).ok_or(CodingError::InvalidRequest)?,
        profile.toolchain_reference().map(str::to_owned),
        profile.environment_profile_reference().into(),
    )
    .map_err(CodingError::owner)?;
    if let Some(format) = profile.diagnostic_format() {
        invocation = invocation.with_diagnostic_format(format);
    }
    Ok(invocation)
}

pub(super) fn process_facts(receipt: &saya_harness::coding::checks::CheckReceipt) -> Value {
    let process = receipt.process();
    json!({"pid":process.pid(),"exit_code":process.exit_code(),"signal":process.signal(),"timed_out":process.timed_out(),"cancelled":process.cancelled(),"killed_orphans":process.killed_orphans(),"joined":true})
}
