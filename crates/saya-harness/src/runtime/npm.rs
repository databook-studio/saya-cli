//! Lock-bound npm input acquisition and closed native check actions.

use std::{path::Path, time::Duration};

use saya_agent::CancellationToken;
use saya_types::Sha256Digest;

use crate::{fetch::FetchTransport, workspace::Workspace};

use super::{
    npm_bundle, npm_check, npm_install, npm_lock,
    npm_receipt::{NpmCheckAction, NpmCheckOutcome, NpmInputBundleReceipt},
    registry::{Registry, RegistryError},
};

/// Explicit local consent to acquire lock-authorized npm tarballs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NpmInstallConsent {
    Granted,
}

/// Public-byte identity of the exact package manifest and v3 lock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NpmProjectBinding {
    pub(super) manifest_sha256: String,
    pub(super) lock_sha256: String,
    pub(super) input_key_sha256: String,
    pub(super) tarball_count: usize,
}

/// Identity recomputed from a verified installed npm input bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedNpmToolchainIdentity {
    input_key_sha256: Sha256Digest,
    bundle_sha256: Sha256Digest,
}

impl VerifiedNpmToolchainIdentity {
    pub fn input_key_sha256(&self) -> &Sha256Digest {
        &self.input_key_sha256
    }

    pub fn bundle_sha256(&self) -> &Sha256Digest {
        &self.bundle_sha256
    }

    pub fn effective_toolchain_id(&self) -> String {
        super::npm_bundle::effective_toolchain_id(
            self.input_key_sha256.as_str(),
            self.bundle_sha256.as_str(),
        )
    }
}

/// Selects the source fingerprint policy for a closed npm check; this does not
/// grant checkout authority. Operator projects admit exact root Git metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NpmCheckSource {
    /// A verified managed checkout; candidate/delta repository rules apply.
    ManagedCandidate,
    /// A user-selected ordinary project; root repository metadata is excluded.
    OperatorProject,
}

impl NpmProjectBinding {
    pub fn manifest_sha256(&self) -> &str {
        &self.manifest_sha256
    }

    pub fn lock_sha256(&self) -> &str {
        &self.lock_sha256
    }

    pub fn input_key_sha256(&self) -> &str {
        &self.input_key_sha256
    }

    pub fn tarball_count(&self) -> usize {
        self.tarball_count
    }
}

/// A source identity supplied by the trusted caller after checkout authority
/// and source verification. The runtime itself accepts no checkout authority.
pub struct NpmCheckRequest<'a> {
    pub state_root: &'a Path,
    pub project: &'a Workspace,
    pub source_sha256: &'a str,
    pub source: NpmCheckSource,
    pub action: NpmCheckAction,
    pub timeout: Duration,
    pub cancellation: &'a CancellationToken,
}

/// Why a lock-bound npm action could not return trustworthy owner facts.
#[derive(Debug, thiserror::Error, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum NpmRuntimeError {
    #[error("explicit npm dependency consent is required")]
    ConsentRequired,
    #[error("the pinned Node runtime is not already installed and verified")]
    NodeRuntimeNotReady,
    #[error("the project package manifest is missing or invalid")]
    InvalidManifest,
    #[error("a supported package-lock.json is required")]
    InvalidLock,
    #[error("the package lock uses an unsupported format")]
    UnsupportedLock,
    #[error("the package lock exceeds its bound")]
    LockTooLarge,
    #[error("the lock includes an unapproved package origin")]
    UntrustedOrigin,
    #[error("every package tarball requires valid SHA-512 SRI")]
    InvalidIntegrity,
    #[error("the package lock exceeds its package or tarball count bound")]
    TooManyPackages,
    #[error("the npm input download exceeds its byte bound")]
    InputTooLarge,
    #[error("the package manifest and lock root dependencies do not match")]
    ManifestLockMismatch,
    #[error("the project source path is invalid for a managed container mount")]
    InvalidProject,
    #[error("project npm configuration is not permitted")]
    NpmrcRefused,
    #[error("preparing the bounded project source exceeded its time budget")]
    PreparationTimedOut,
    #[error("the lock-bound npm input bundle is not installed")]
    InputBundleMissing,
    #[error("the retained managed runtime record is not the pinned Node/npm runtime")]
    StateConflict,
    #[error("an earlier managed container probe must be recovered first")]
    RecoveryRequired,
    #[error("the project changed between its supplied identity and the runtime snapshot")]
    SourceChanged,
    #[error("the npm operation was cancelled")]
    Cancelled,
    #[error("npm input acquisition or container execution failed")]
    Failed,
    #[error("offline dependency validation failed after the container finished")]
    ValidationFailed,
    #[error("offline dependency validation was cancelled after the container finished")]
    ValidationCancelled,
    #[error("the container outcome or cleanup is unknown")]
    Unknown,
}

pub(super) fn open_ready_registry(state_root: &Path) -> Result<Registry, NpmRuntimeError> {
    let registry = Registry::open(state_root).map_err(registry_error)?;
    ensure_ready_registry(&registry)?;
    Ok(registry)
}

pub(super) fn ensure_ready_registry(registry: &Registry) -> Result<(), NpmRuntimeError> {
    if !registry
        .pending_probe_tickets()
        .map_err(registry_error)?
        .is_empty()
    {
        return Err(NpmRuntimeError::RecoveryRequired);
    }
    if registry.has_ready().map_err(registry_error)? {
        Ok(())
    } else {
        Err(NpmRuntimeError::NodeRuntimeNotReady)
    }
}

pub(super) fn registry_error(error: RegistryError) -> NpmRuntimeError {
    match error {
        RegistryError::InvalidStateRoot | RegistryError::Io => NpmRuntimeError::Failed,
        #[cfg(unix)]
        RegistryError::Busy => NpmRuntimeError::Failed,
        RegistryError::Conflict => NpmRuntimeError::StateConflict,
    }
}

/// Reads and validates the package manifest/lock binding without fetching.
pub fn bind_npm_project(project: &Workspace) -> Result<NpmProjectBinding, NpmRuntimeError> {
    let binding = npm_lock::bind_project_inputs(project)?;
    Ok(NpmProjectBinding {
        manifest_sha256: binding.manifest_sha256().to_owned(),
        lock_sha256: binding.lock_sha256().to_owned(),
        input_key_sha256: binding.input_key_sha256().to_owned(),
        tarball_count: binding.tarballs().len(),
    })
}

/// Recomputes a managed npm identity from a ready registry and a bounded project binding.
/// This reads and verifies existing state; it never fetches, installs, or starts a process.
pub fn verified_npm_toolchain_identity(
    state_root: &Path,
    project: &Workspace,
) -> Result<VerifiedNpmToolchainIdentity, NpmRuntimeError> {
    let binding = npm_lock::bind_project_inputs(project)?;
    let registry = open_ready_registry(state_root)?;
    let bundle = npm_bundle::load_verified(&registry, &binding)?
        .ok_or(NpmRuntimeError::InputBundleMissing)?;
    let input_key_sha256 = Sha256Digest::parse(binding.input_key_sha256())
        .map_err(|_| NpmRuntimeError::StateConflict)?;
    let bundle_sha256 =
        Sha256Digest::parse(&bundle.bundle_sha256).map_err(|_| NpmRuntimeError::StateConflict)?;
    Ok(VerifiedNpmToolchainIdentity {
        input_key_sha256,
        bundle_sha256,
    })
}

/// Acquires only SRI-authorized tarballs for a validated project workspace.
/// The pinned Node runtime must already be ready; this operation never pulls it.
pub async fn install_npm_inputs(
    state_root: &Path,
    project: &Workspace,
    consent: Option<NpmInstallConsent>,
    transport: &dyn FetchTransport,
    cancellation: &CancellationToken,
) -> Result<NpmInputBundleReceipt, NpmRuntimeError> {
    npm_install::install(state_root, project, consent, transport, cancellation).await
}

/// Recreates dependencies in bounded disposable storage and runs one closed
/// npm action. The caller owns checkout authority and source-before/after proof.
pub async fn run_npm_check(
    request: NpmCheckRequest<'_>,
) -> Result<NpmCheckOutcome, NpmRuntimeError> {
    npm_check::run(request).await
}
