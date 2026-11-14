use super::{CodingError, checks::ChildCodingOperations};
use saya_agent::CancellationToken;
use saya_config::ExtensionProtocol;
use saya_harness::{
    coding::{extensions::ExtensionConfig, worktree::verify_owned_checkout},
    runner::sandbox::{RunSandbox, SandboxProvision},
};
use std::path::PathBuf;
impl ChildCodingOperations {
    pub(super) async fn extension(
        &self,
        profile: &str,
        protocol: ExtensionProtocol,
        token: &CancellationToken,
    ) -> Result<(ExtensionConfig, saya_config::ExtensionProfile, PathBuf), CodingError> {
        let profile = self
            .application
            .runtime
            .resolved
            .coding
            .extensions
            .get(profile)
            .filter(|profile| profile.protocol() == protocol)
            .ok_or(CodingError::InvalidRequest)?
            .clone();
        let verified = verify_owned_checkout(&self.ownership()?, token)
            .await
            .map_err(CodingError::owner)?;
        let root = PathBuf::from(verified.path().as_str());
        let sandbox =
            RunSandbox::new([root.clone()], std::iter::empty()).map_err(CodingError::owner)?;
        // The startup probe measures this host with real sandboxed children;
        // that measurement is synchronous and slow, so it runs on the
        // blocking pool and never stalls the job's runtime.
        let program_dir = self.application.program_dir.clone();
        let spawn = match tokio::task::spawn_blocking(move || sandbox.prepare(&program_dir))
            .await
            .map_err(CodingError::owner)?
            .map_err(CodingError::owner)?
        {
            SandboxProvision::Proven { spawn, .. } => spawn,
            _ => return Err(CodingError::ConfinementUnavailable),
        };
        let config = ExtensionConfig::new(
            spawn,
            self.application.program_dir.join(profile.program()),
            profile.argv().to_vec(),
        )
        // Checked before any server process exists; the usual cause is a
        // symlinked program (a distribution's `sh` or `python3`).
        .map_err(|_| CodingError::ExtensionProgramRefused)?;
        let config = config.with_observer(std::sync::Arc::new(
            super::extension_observer::ProcessObserver::new(self),
        ));
        // No inherited credentials and no implicit secret injection. A program
        // grant is not a secret grant; profile data never widens the root.
        Ok((config, profile, root))
    }
}
