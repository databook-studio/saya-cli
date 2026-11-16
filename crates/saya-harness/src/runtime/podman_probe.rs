//! Fixed Node version probes and recoverable container ownership.

use std::{fs, path::Path, time::Instant};

use saya_agent::CancellationToken;

use super::super::{
    CleanupStatus, PINNED_NODE_IMAGE,
    podman_cleanup::{CLEANUP_BUDGET, CleanupContext, cleanup},
    podman_process::ticket,
    registry::Registry,
};
use super::{COMMAND_TIMEOUT, Podman, PodmanError, ProbeFailure};

const LABEL: &str = "io.saya.managed-node-ticket";

impl Podman {
    pub(crate) async fn probe_node(
        &self,
        registry: &Registry,
        cancellation: &CancellationToken,
    ) -> Result<String, PodmanError> {
        self.probe(
            registry,
            &["/usr/local/bin/node", "--version"],
            cancellation,
        )
        .await
    }

    pub(crate) async fn probe_npm(
        &self,
        registry: &Registry,
        cancellation: &CancellationToken,
    ) -> Result<String, PodmanError> {
        self.probe(
            registry,
            &[
                "/usr/local/bin/node",
                "/usr/local/lib/node_modules/npm/bin/npm-cli.js",
                "--version",
            ],
            cancellation,
        )
        .await
    }

    async fn probe(
        &self,
        registry: &Registry,
        program: &[&str],
        cancellation: &CancellationToken,
    ) -> Result<String, PodmanError> {
        let ticket = ticket()?;
        registry
            .begin_probe(&ticket)
            .map_err(|_| PodmanError::Failed)?;
        let name = container_name(&ticket);
        let root = registry.root();
        let cidfile = root.join(format!("container-{ticket}"));
        let cidfile_arg = cidfile.to_str().ok_or(PodmanError::Failed)?;
        let label = format!("{LABEL}={ticket}");
        let mut args = vec![
            "run",
            "--rm",
            "--pull=never",
            "--cidfile",
            cidfile_arg,
            "--name",
            &name,
            "--label",
            &label,
            "--network",
            "none",
            "--read-only",
            "--cap-drop",
            "all",
            "--security-opt",
            "no-new-privileges",
            "--pids-limit",
            "64",
            "--memory",
            "512m",
            "--cpus",
            "1",
            "--env",
            "HOME=/nonexistent",
            "--env",
            "PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
            "--env",
            "NODE_VERSION=",
            "--env",
            "YARN_VERSION=",
            PINNED_NODE_IMAGE.reference(),
        ];
        args.extend(program);
        let result = self.success(&args, COMMAND_TIMEOUT, cancellation).await;
        self.finish_probe(result, registry, &cidfile, &name, &ticket)
            .await
            .map(|value| value.trim().to_owned())
    }

    async fn finish_probe(
        &self,
        result: Result<String, PodmanError>,
        registry: &Registry,
        cidfile: &Path,
        name: &str,
        ticket: &str,
    ) -> Result<String, PodmanError> {
        let cleanup_cancellation = CancellationToken::new();
        let cleanup_context = CleanupContext::new(
            &self.executable,
            self.home.as_deref(),
            self.authfile()?,
            Instant::now() + CLEANUP_BUDGET,
            &cleanup_cancellation,
        );
        let cleanup = cleanup(&cleanup_context, cidfile, name, ticket, result.is_ok()).await;
        match (result, cleanup) {
            (Ok(output), CleanupStatus::NotNeeded | CleanupStatus::Removed) => {
                self.finish_locator(registry, cidfile, ticket, None)?;
                Ok(output)
            }
            (Err(error), CleanupStatus::NotNeeded | CleanupStatus::Removed) => {
                self.finish_locator(registry, cidfile, ticket, Some(error))?;
                Err(error)
            }
            (result, CleanupStatus::Unknown) => Err(PodmanError::CleanupUnknown {
                operation: result.err().and_then(probe_failure),
            }),
        }
    }

    pub(super) fn finish_locator(
        &self,
        registry: &Registry,
        cidfile: &Path,
        ticket: &str,
        operation: Option<PodmanError>,
    ) -> Result<(), PodmanError> {
        match fs::remove_file(cidfile) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => {
                return Err(PodmanError::CleanupUnknown {
                    operation: operation.and_then(probe_failure),
                });
            }
        }
        registry
            .finish_probe(ticket)
            .map_err(|_| PodmanError::CleanupUnknown {
                operation: operation.and_then(probe_failure),
            })
    }
}

pub(super) fn container_name(ticket: &str) -> String {
    format!("saya-node-probe-{ticket}")
}

fn probe_failure(error: PodmanError) -> Option<ProbeFailure> {
    match error {
        PodmanError::Unavailable => Some(ProbeFailure::Unavailable),
        PodmanError::Failed => Some(ProbeFailure::Failed),
        PodmanError::Cancelled => Some(ProbeFailure::Cancelled),
        PodmanError::TimedOut => Some(ProbeFailure::TimedOut),
        PodmanError::OutputLimited => Some(ProbeFailure::OutputLimited),
        PodmanError::CleanupUnknown { operation } => operation,
    }
}
