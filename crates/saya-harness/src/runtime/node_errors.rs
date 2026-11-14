//! Public installation phases and uncertainty-preserving errors.

use thiserror::Error;

/// Whether removal of an installer-owned container was proved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CleanupStatus {
    NotNeeded,
    Removed,
    Unknown,
}

/// The fixed installer phase reached before failure or cancellation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeInstallPhase {
    Pull,
    AttestImage,
    ReconcileProbe,
    ProbeNode,
    ProbeNpm,
    Publish,
}

/// A managed-runtime install that did not prove a reusable ready record.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum NodeInstallError {
    #[error("managed Node installation requires explicit consent")]
    ConsentRequired,
    #[error("the managed Node runtime is unsupported on this platform")]
    UnsupportedPlatform,
    #[error("the trusted Podman engine is unavailable")]
    EngineUnavailable,
    #[error("the private managed-runtime state root is unsafe or unavailable")]
    InvalidStateRoot,
    #[error("the private managed-runtime registry is busy")]
    Busy,
    #[error(
        "an existing managed-runtime record was retained because it is not the compiled runtime"
    )]
    ConflictRetained,
    #[error(
        "Podman did not prove the required Node image: observed {observed}; image cache may have changed: {image_cache_may_have_changed}"
    )]
    ImageUnproved {
        observed: String,
        image_cache_may_have_changed: bool,
    },
    #[error(
        "Podman did not prove the required Node version: observed {observed}; image cache may have changed: {image_cache_may_have_changed}; cleanup: {cleanup:?}"
    )]
    NodeVersionUnproved {
        observed: String,
        image_cache_may_have_changed: bool,
        cleanup: CleanupStatus,
    },
    #[error(
        "Podman did not prove the required npm version: observed {observed}; image cache may have changed: {image_cache_may_have_changed}; cleanup: {cleanup:?}"
    )]
    NpmVersionUnproved {
        observed: String,
        image_cache_may_have_changed: bool,
        cleanup: CleanupStatus,
    },
    #[error(
        "managed Node {phase:?} failed; image cache may have changed: {image_cache_may_have_changed}; cleanup: {cleanup:?}"
    )]
    OperationFailed {
        phase: NodeInstallPhase,
        image_cache_may_have_changed: bool,
        cleanup: CleanupStatus,
    },
    #[error(
        "managed Node {phase:?} was cancelled; image cache may have changed: {image_cache_may_have_changed}; cleanup: {cleanup:?}"
    )]
    Cancelled {
        phase: NodeInstallPhase,
        image_cache_may_have_changed: bool,
        cleanup: CleanupStatus,
    },
    #[error("managed Node publication could not be proved; the record was retained")]
    PublicationUnproved,
}
