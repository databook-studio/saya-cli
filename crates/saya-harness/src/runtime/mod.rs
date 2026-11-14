//! A private, digest-pinned Node/npm runtime for confined project checks.

mod node;
mod node_errors;
mod npm;
mod npm_bundle;
mod npm_bundle_receipt;
mod npm_check;
mod npm_check_receipt;
mod npm_install;
mod npm_install_receipt;
mod npm_lock;
mod npm_receipt;
mod npm_snapshot;
mod podman;
mod podman_cleanup;
mod podman_process;
mod registry;

use std::path::Path;

use saya_agent::CancellationToken;

use self::{
    podman::{Podman, PodmanError, ProbeFailure},
    registry::{Registry, RegistryError},
};

/// The sticky bit is `0o1000` on every Unix libc; `libc::S_ISVTX` is `u32` on
/// Linux but `u16` on macOS, so name the portable value once.
#[cfg(unix)]
pub(crate) const STICKY_BIT: u32 = 0o1000;

pub use node::{
    NODE_IMAGE_MANIFEST_DIGEST, NODE_IMAGE_REFERENCE, NODE_IMAGE_REPOSITORY, NODE_IMAGE_TAG,
    NODE_VERSION, NPM_VERSION, NodeImageReference, NodeInstallConsent, NodeInstallReceipt,
    PINNED_NODE_IMAGE,
};
pub use node_errors::{CleanupStatus, NodeInstallError, NodeInstallPhase};
pub use npm::{
    NpmCheckRequest, NpmCheckSource, NpmInstallConsent, NpmProjectBinding, NpmRuntimeError,
    VerifiedNpmToolchainIdentity, bind_npm_project, install_npm_inputs, run_npm_check,
    verified_npm_toolchain_identity,
};
pub use npm_receipt::{NpmCheckAction, NpmCheckOutcome, NpmCheckStatus, NpmInputBundleReceipt};

/// Observes the exact private installation record without modifying state or
/// starting the engine. Image presence and execution still need revalidation.
pub fn node_installation_recorded(state_root: &Path) -> Result<bool, NodeInstallError> {
    if !supported_platform() {
        return Err(NodeInstallError::UnsupportedPlatform);
    }
    Registry::installation_recorded(state_root).map_err(registry_error)
}

/// Installs the compiled managed Node runtime under the Saya-owned state root.
/// The facade accepts no image, version, program, registry, or destination.
pub async fn install_node(
    state_root: &Path,
    consent: Option<NodeInstallConsent>,
    cancellation: &CancellationToken,
) -> Result<NodeInstallReceipt, NodeInstallError> {
    if consent.is_none() {
        return Err(NodeInstallError::ConsentRequired);
    }
    if !supported_platform() {
        return Err(NodeInstallError::UnsupportedPlatform);
    }
    let mut podman = Podman::discover().map_err(podman_unavailable)?;
    let registry = Registry::open(state_root).map_err(registry_error)?;
    podman.bind_authfile(registry.authfile());
    install_with(registry, podman, cancellation).await
}

async fn install_with(
    registry: Registry,
    podman: Podman,
    cancellation: &CancellationToken,
) -> Result<NodeInstallReceipt, NodeInstallError> {
    podman
        .reconcile_pending(&registry, cancellation)
        .await
        .map_err(|error| operation_error(error, NodeInstallPhase::ReconcileProbe, false))?;
    let reused = registry.has_ready().map_err(registry_error)?;
    let digest = if reused {
        podman
            .inspect(cancellation)
            .await
            .map_err(|error| operation_error(error, NodeInstallPhase::AttestImage, false))?
    } else {
        podman
            .pull(cancellation)
            .await
            .map_err(|error| operation_error(error, NodeInstallPhase::Pull, true))?;
        podman
            .inspect(cancellation)
            .await
            .map_err(|error| operation_error(error, NodeInstallPhase::AttestImage, true))?
    };
    if digest != NODE_IMAGE_MANIFEST_DIGEST {
        return Err(NodeInstallError::ImageUnproved {
            observed: digest,
            image_cache_may_have_changed: !reused,
        });
    }
    let node = podman
        .probe_node(&registry, cancellation)
        .await
        .map_err(|error| operation_error(error, NodeInstallPhase::ProbeNode, !reused))?;
    if node != NODE_VERSION {
        return Err(NodeInstallError::NodeVersionUnproved {
            observed: node,
            image_cache_may_have_changed: !reused,
            cleanup: CleanupStatus::NotNeeded,
        });
    }
    let npm = podman
        .probe_npm(&registry, cancellation)
        .await
        .map_err(|error| operation_error(error, NodeInstallPhase::ProbeNpm, !reused))?;
    if npm != NPM_VERSION {
        return Err(NodeInstallError::NpmVersionUnproved {
            observed: npm,
            image_cache_may_have_changed: !reused,
            cleanup: CleanupStatus::NotNeeded,
        });
    }
    if reused {
        return Ok(NodeInstallReceipt::Reused {
            image: PINNED_NODE_IMAGE,
        });
    }
    if cancellation.is_cancelled() {
        return Err(NodeInstallError::Cancelled {
            phase: NodeInstallPhase::Publish,
            image_cache_may_have_changed: true,
            cleanup: CleanupStatus::NotNeeded,
        });
    }
    registry
        .publish()
        .map_err(|_| NodeInstallError::PublicationUnproved)?;
    Ok(NodeInstallReceipt::Installed {
        image: PINNED_NODE_IMAGE,
    })
}

fn supported_platform() -> bool {
    cfg!(all(
        target_arch = "aarch64",
        any(target_os = "macos", target_os = "linux")
    ))
}

fn podman_unavailable(error: PodmanError) -> NodeInstallError {
    match error {
        PodmanError::Unavailable => NodeInstallError::EngineUnavailable,
        _ => NodeInstallError::OperationFailed {
            phase: NodeInstallPhase::AttestImage,
            image_cache_may_have_changed: false,
            cleanup: CleanupStatus::NotNeeded,
        },
    }
}

fn registry_error(error: RegistryError) -> NodeInstallError {
    match error {
        RegistryError::InvalidStateRoot | RegistryError::Io => NodeInstallError::InvalidStateRoot,
        #[cfg(unix)]
        RegistryError::Busy => NodeInstallError::Busy,
        RegistryError::Conflict => NodeInstallError::ConflictRetained,
    }
}

fn operation_error(
    error: PodmanError,
    phase: NodeInstallPhase,
    image_cache_may_have_changed: bool,
) -> NodeInstallError {
    let (cleanup, cancellation) = match error {
        PodmanError::CleanupUnknown { operation } => (
            CleanupStatus::Unknown,
            operation == Some(ProbeFailure::Cancelled),
        ),
        PodmanError::Cancelled => (CleanupStatus::NotNeeded, true),
        _ => (CleanupStatus::NotNeeded, false),
    };
    if cancellation {
        NodeInstallError::Cancelled {
            phase,
            image_cache_may_have_changed,
            cleanup,
        }
    } else {
        NodeInstallError::OperationFailed {
            phase,
            image_cache_may_have_changed,
            cleanup,
        }
    }
}

#[cfg(test)]
mod tests;
