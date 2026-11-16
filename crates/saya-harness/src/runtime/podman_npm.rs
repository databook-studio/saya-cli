//! Fixed, no-network Node actions over verified read-only inputs.

use std::{path::Path, time::Instant};

use saya_agent::CancellationToken;

use super::super::{
    CleanupStatus,
    podman_cleanup::{CLEANUP_BUDGET, CleanupContext, cleanup},
    podman_process::{Output, run},
    registry::Registry,
};
use super::{PINNED_NODE_IMAGE, Podman, PodmanError, ProbeFailure, probe::container_name};

const LABEL: &str = "io.saya.managed-node-ticket";
const MAX_NPM_ACTION_TIME: std::time::Duration = std::time::Duration::from_secs(600);

pub(in crate::runtime) struct NpmActionRequest<'a> {
    pub(in crate::runtime) ticket: &'a str,
    pub(in crate::runtime) bundle: &'a Path,
    pub(in crate::runtime) controller: &'a Path,
    pub(in crate::runtime) source: Option<&'a Path>,
    pub(in crate::runtime) action: NodeAction,
    pub(in crate::runtime) timeout: std::time::Duration,
    pub(in crate::runtime) cancellation: &'a CancellationToken,
}

#[derive(Clone, Copy)]
pub(in crate::runtime) enum NodeAction {
    InstallDeps,
    TestCi,
    Build,
}

impl NodeAction {
    fn marker(self) -> &'static str {
        match self {
            Self::InstallDeps => "install-deps",
            Self::TestCi => "test-ci",
            Self::Build => "build",
        }
    }

    fn argument(self) -> &'static str {
        self.marker()
    }
}

impl Podman {
    pub(in crate::runtime) async fn run_npm_action(
        &self,
        registry: &Registry,
        request: NpmActionRequest<'_>,
    ) -> Result<(Output, CleanupStatus), PodmanError> {
        let NpmActionRequest {
            ticket,
            bundle,
            controller,
            source,
            action,
            timeout,
            cancellation,
        } = request;
        let bundle = bundle.to_str().ok_or(PodmanError::Failed)?;
        let controller = controller.to_str().ok_or(PodmanError::Failed)?;
        let source = source
            .map(|path| path.to_str().ok_or(PodmanError::Failed))
            .transpose()?;
        let authfile = self.authfile()?;
        let name = container_name(ticket);
        let cidfile = registry.root().join(format!("container-{ticket}"));
        let cidfile_arg = cidfile.to_str().ok_or(PodmanError::Failed)?;
        let label = format!("{LABEL}={ticket}");
        let mut args = vec![
            "run".to_owned(),
            "--rm".to_owned(),
            "--pull=never".to_owned(),
            "--cidfile".to_owned(),
            cidfile_arg.to_owned(),
            "--name".to_owned(),
            name.clone(),
            "--label".to_owned(),
            label,
            "--network".to_owned(),
            "none".to_owned(),
            "--read-only".to_owned(),
            "--cap-drop".to_owned(),
            "all".to_owned(),
            "--security-opt".to_owned(),
            "no-new-privileges".to_owned(),
            "--pids-limit".to_owned(),
            "64".to_owned(),
            "--memory".to_owned(),
            "1g".to_owned(),
            "--cpus".to_owned(),
            "1".to_owned(),
            "--tmpfs".to_owned(),
            "/saya/work:rw,size=512m".to_owned(),
            "--tmpfs".to_owned(),
            "/saya/cache:rw,size=256m".to_owned(),
            "--tmpfs".to_owned(),
            "/tmp:rw,nosuid,size=64m".to_owned(),
            "--volume".to_owned(),
            format!("{bundle}:/saya/bundle:ro"),
            "--volume".to_owned(),
            format!("{controller}:/saya/npm-controller.js:ro"),
            "--env".to_owned(),
            "HOME=/saya/work/home".to_owned(),
            "--env".to_owned(),
            "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin".to_owned(),
            "--env".to_owned(),
            "NPM_CONFIG_CACHE=/saya/cache".to_owned(),
            "--env".to_owned(),
            "NPM_CONFIG_USERCONFIG=/saya/work/user.npmrc".to_owned(),
            "--env".to_owned(),
            "NPM_CONFIG_GLOBALCONFIG=/saya/work/global.npmrc".to_owned(),
            "--env".to_owned(),
            "NPM_CONFIG_AUDIT=false".to_owned(),
            "--env".to_owned(),
            "NPM_CONFIG_FUND=false".to_owned(),
            "--env".to_owned(),
            format!("SAYA_NODE_ACTION={}", action.marker()),
        ];
        if let Some(source) = source {
            args.push("--volume".to_owned());
            args.push(format!("{source}:/saya/source:ro"));
        }
        args.push(PINNED_NODE_IMAGE.reference().to_owned());
        args.push("/usr/local/bin/node".to_owned());
        args.push("/saya/npm-controller.js".to_owned());
        args.push(action.argument().to_owned());
        let references: Vec<&str> = args.iter().map(String::as_str).collect();
        let result = run(
            &self.executable,
            self.home.as_deref(),
            authfile,
            &references,
            timeout.min(MAX_NPM_ACTION_TIME),
            cancellation,
        )
        .await;
        self.finish_npm_action(result, &cidfile, &name, ticket)
            .await
    }

    pub(in crate::runtime) fn finish_npm_ticket(
        &self,
        registry: &Registry,
        ticket: &str,
    ) -> Result<(), PodmanError> {
        let cidfile = registry.root().join(format!("container-{ticket}"));
        self.finish_locator(registry, &cidfile, ticket, None)
    }

    async fn finish_npm_action(
        &self,
        result: Result<Output, PodmanError>,
        cidfile: &Path,
        name: &str,
        ticket: &str,
    ) -> Result<(Output, CleanupStatus), PodmanError> {
        if let Err(PodmanError::CleanupUnknown { operation }) = &result {
            let _ = self.cleanup_action(cidfile, name, ticket, false).await;
            return Err(PodmanError::CleanupUnknown {
                operation: *operation,
            });
        }
        let completed = matches!(
            &result,
            Ok(output)
                if !output.timed_out
                    && !output.cancelled
                    && !output.killed_orphans
                    && output
                        .exit_code
                        .is_some_and(|code| !matches!(code, 125..=127))
        );
        let cleanup = self.cleanup_action(cidfile, name, ticket, completed).await;
        match (result, cleanup) {
            (Ok(output), CleanupStatus::NotNeeded | CleanupStatus::Removed) => {
                Ok((output, cleanup))
            }
            (Err(error), CleanupStatus::NotNeeded | CleanupStatus::Removed) => Err(error),
            (result, CleanupStatus::Unknown) => Err(PodmanError::CleanupUnknown {
                operation: match result {
                    Ok(output) if output.cancelled => Some(ProbeFailure::Cancelled),
                    Ok(output) if output.timed_out => Some(ProbeFailure::TimedOut),
                    Ok(_) => None,
                    Err(PodmanError::Unavailable) => None,
                    Err(_) => Some(ProbeFailure::Failed),
                },
            }),
        }
    }

    async fn cleanup_action(
        &self,
        cidfile: &Path,
        name: &str,
        ticket: &str,
        completed: bool,
    ) -> CleanupStatus {
        let cancellation = CancellationToken::new();
        let Ok(authfile) = self.authfile() else {
            return CleanupStatus::Unknown;
        };
        let context = CleanupContext::new(
            &self.executable,
            self.home.as_deref(),
            authfile,
            Instant::now() + CLEANUP_BUDGET,
            &cancellation,
        );
        cleanup(&context, cidfile, name, ticket, completed).await
    }
}
