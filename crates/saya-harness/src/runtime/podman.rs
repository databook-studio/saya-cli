//! Fixed-path Podman commands and container ownership decisions.

#[path = "podman_discovery.rs"]
mod discovery;
#[path = "podman_npm.rs"]
mod npm;
#[path = "podman_probe.rs"]
mod probe;
#[path = "podman_recovery.rs"]
mod recovery;

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use saya_agent::CancellationToken;

pub(super) use super::podman_process::Output;
use super::{PINNED_NODE_IMAGE, podman_process::run};
pub(super) use npm::{NodeAction, NpmActionRequest};

const PULL_TIMEOUT: Duration = Duration::from_secs(120);
pub(super) const COMMAND_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PodmanError {
    Unavailable,
    Failed,
    Cancelled,
    TimedOut,
    OutputLimited,
    CleanupUnknown { operation: Option<ProbeFailure> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProbeFailure {
    Unavailable,
    Failed,
    Cancelled,
    TimedOut,
    OutputLimited,
}

pub(crate) struct Podman {
    executable: PathBuf,
    home: Option<PathBuf>,
    authfile: Option<PathBuf>,
}

impl Podman {
    pub(crate) fn discover() -> Result<Self, PodmanError> {
        discovery::candidates()
            .into_iter()
            .find_map(|candidate| Self::from_candidate(candidate, None))
            .ok_or(PodmanError::Unavailable)
    }

    pub(super) fn from_candidate(executable: PathBuf, authfile: Option<PathBuf>) -> Option<Self> {
        let executable = discovery::trusted_executable(&executable)?;
        let home = discovery::trusted_home();
        Some(Self {
            executable,
            home,
            authfile,
        })
    }

    pub(crate) fn bind_authfile(&mut self, authfile: PathBuf) {
        self.authfile = Some(authfile);
    }

    fn authfile(&self) -> Result<&Path, PodmanError> {
        self.authfile.as_deref().ok_or(PodmanError::Failed)
    }

    pub(crate) async fn pull(&self, cancellation: &CancellationToken) -> Result<(), PodmanError> {
        let authfile = self.authfile()?.to_str().ok_or(PodmanError::Failed)?;
        self.success(
            &[
                "pull",
                "--quiet",
                "--authfile",
                authfile,
                PINNED_NODE_IMAGE.reference(),
            ],
            PULL_TIMEOUT,
            cancellation,
        )
        .await
        .map(|_| ())
    }

    pub(crate) async fn inspect(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<String, PodmanError> {
        let output = self
            .success(
                &[
                    "image",
                    "inspect",
                    "--format",
                    "{{.Digest}}",
                    PINNED_NODE_IMAGE.reference(),
                ],
                COMMAND_TIMEOUT,
                cancellation,
            )
            .await?;
        Ok(output.trim().to_owned())
    }

    async fn success(
        &self,
        args: &[&str],
        timeout: Duration,
        cancellation: &CancellationToken,
    ) -> Result<String, PodmanError> {
        let output = run(
            &self.executable,
            self.home.as_deref(),
            self.authfile()?,
            args,
            timeout,
            cancellation,
        )
        .await?;
        if output.cancelled {
            Err(PodmanError::Cancelled)
        } else if output.timed_out {
            Err(PodmanError::TimedOut)
        } else if output.stdout.truncated || output.stderr.truncated {
            Err(PodmanError::OutputLimited)
        } else if output.success {
            Ok(output.stdout.text)
        } else {
            Err(PodmanError::Failed)
        }
    }
}
