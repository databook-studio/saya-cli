//! Lock-bound input fetching, offline validation, and no-replace publication.

use std::{path::Path, time::Duration};

use saya_agent::CancellationToken;
use tokio::time::Instant;

use crate::fetch::{
    DownloadBudget, DownloadCall, DownloadError, DownloadLimits, FetchDestination, FetchPolicy,
    FetchTransport, http_download_cancellable,
};

use super::{
    NpmInstallConsent, NpmRuntimeError,
    npm::{open_ready_registry, registry_error},
    npm_bundle,
    npm_install_receipt::bundle_receipt,
    npm_lock::{self, NpmInputBinding},
    npm_receipt::NpmInputBundleReceipt,
    podman::{NodeAction, NpmActionRequest, Podman, PodmanError},
    podman_process::ticket,
    registry::Registry,
};

const MAX_INPUT_TIME: Duration = Duration::from_secs(600);
const REQUEST_TIME: Duration = Duration::from_secs(60);

pub(super) async fn install(
    state_root: &Path,
    project: &crate::workspace::Workspace,
    consent: Option<NpmInstallConsent>,
    transport: &dyn FetchTransport,
    cancellation: &CancellationToken,
) -> Result<NpmInputBundleReceipt, NpmRuntimeError> {
    if consent.is_none() {
        return Err(NpmRuntimeError::ConsentRequired);
    }
    if cancellation.is_cancelled() {
        return Err(NpmRuntimeError::Cancelled);
    }
    npm_lock::validate_mount_path(state_root)?;
    let binding = npm_lock::bind_project_inputs(project)?;
    let registry = open_ready_registry(state_root)?;
    let mut podman = Podman::discover().map_err(|_| NpmRuntimeError::Failed)?;
    podman.bind_authfile(registry.authfile());
    install_with(registry, podman, binding, transport, cancellation).await
}

pub(super) async fn install_with(
    registry: Registry,
    podman: Podman,
    binding: NpmInputBinding,
    transport: &dyn FetchTransport,
    cancellation: &CancellationToken,
) -> Result<NpmInputBundleReceipt, NpmRuntimeError> {
    let started = Instant::now();
    npm_bundle::cleanup_abandoned_stages(&registry)?;
    let existing = npm_bundle::load_verified(&registry, &binding)?;
    let operation_ticket = ticket().map_err(|_| NpmRuntimeError::Failed)?;
    let stage = registry
        .create_npm_stage(&operation_ticket)
        .map_err(registry_error)?;
    let (bundle_path, controller, new_bundle) = match existing {
        Some(bundle) => {
            let controller = match npm_bundle::write_controller(&stage) {
                Ok(controller) => controller,
                Err(error) => {
                    npm_bundle::cleanup_stage(&registry, &stage)?;
                    return Err(error);
                }
            };
            (bundle.path, controller, false)
        }
        None => {
            if let Err(error) = npm_bundle::create_stage_files(&stage, &binding) {
                npm_bundle::cleanup_stage(&registry, &stage)?;
                return Err(error);
            }
            if let Err(error) =
                acquire_tarballs(&stage, &binding, transport, cancellation, started).await
            {
                npm_bundle::cleanup_stage(&registry, &stage)?;
                return Err(error);
            }
            let entries = match npm_bundle::validate_stage_tarballs(&stage, &binding) {
                Ok(entries) => entries,
                Err(error) => {
                    npm_bundle::cleanup_stage(&registry, &stage)?;
                    return Err(error);
                }
            };
            let manifest = match npm_bundle::bundle_manifest_bytes(&binding, entries) {
                Ok(manifest) => manifest,
                Err(error) => {
                    npm_bundle::cleanup_stage(&registry, &stage)?;
                    return Err(error);
                }
            };
            if let Err(error) = npm_bundle::write_bundle_manifest(&stage, &manifest) {
                npm_bundle::cleanup_stage(&registry, &stage)?;
                return Err(error);
            }
            (stage.clone(), stage.join("npm-controller.js"), true)
        }
    };
    let remaining = MAX_INPUT_TIME.saturating_sub(started.elapsed());
    if remaining.is_zero() {
        npm_bundle::cleanup_stage(&registry, &stage)?;
        return Err(NpmRuntimeError::PreparationTimedOut);
    }
    if cancellation.is_cancelled() {
        npm_bundle::cleanup_stage(&registry, &stage)?;
        return Err(NpmRuntimeError::Cancelled);
    }
    if let Err(error) = registry.begin_probe(&operation_ticket) {
        npm_bundle::cleanup_stage(&registry, &stage)?;
        return Err(registry_error(error));
    }
    let validated = podman
        .run_npm_action(
            &registry,
            NpmActionRequest {
                ticket: &operation_ticket,
                bundle: &bundle_path,
                controller: &controller,
                source: None,
                action: NodeAction::InstallDeps,
                timeout: remaining,
                cancellation,
            },
        )
        .await;
    let (output, cleanup) = match validated {
        Ok(result) => result,
        Err(PodmanError::CleanupUnknown { .. }) => return Err(NpmRuntimeError::Unknown),
        Err(error) => {
            npm_bundle::cleanup_stage(&registry, &stage).map_err(|_| NpmRuntimeError::Unknown)?;
            podman
                .finish_npm_ticket(&registry, &operation_ticket)
                .map_err(|_| NpmRuntimeError::Unknown)?;
            return Err(map_podman_error(error));
        }
    };
    if output.cancelled {
        npm_bundle::cleanup_stage(&registry, &stage).map_err(|_| NpmRuntimeError::Unknown)?;
        podman
            .finish_npm_ticket(&registry, &operation_ticket)
            .map_err(|_| NpmRuntimeError::Unknown)?;
        return Err(NpmRuntimeError::ValidationCancelled);
    }
    if output.timed_out || output.exit_code != Some(0) {
        npm_bundle::cleanup_stage(&registry, &stage).map_err(|_| NpmRuntimeError::Unknown)?;
        podman
            .finish_npm_ticket(&registry, &operation_ticket)
            .map_err(|_| NpmRuntimeError::Unknown)?;
        return Err(NpmRuntimeError::ValidationFailed);
    }
    let bundle = match new_bundle {
        true => {
            npm_bundle::remove_stage_file(&stage, "npm-controller.js")
                .map_err(|_| NpmRuntimeError::Unknown)?;
            registry
                .publish_npm_bundle(&stage, binding.input_key_sha256())
                .map_err(|_| NpmRuntimeError::Unknown)?;
            npm_bundle::load_verified(&registry, &binding)
                .map_err(|_| NpmRuntimeError::Unknown)?
                .ok_or(NpmRuntimeError::Unknown)?
        }
        false => {
            npm_bundle::cleanup_stage(&registry, &stage).map_err(|_| NpmRuntimeError::Unknown)?;
            npm_bundle::load_verified(&registry, &binding)
                .map_err(|_| NpmRuntimeError::Unknown)?
                .ok_or(NpmRuntimeError::Unknown)?
        }
    };
    podman
        .finish_npm_ticket(&registry, &operation_ticket)
        .map_err(|_| NpmRuntimeError::Unknown)?;
    Ok(bundle_receipt(&binding, &bundle, output, cleanup))
}

async fn acquire_tarballs(
    stage: &Path,
    binding: &NpmInputBinding,
    transport: &dyn FetchTransport,
    cancellation: &CancellationToken,
    started: Instant,
) -> Result<(), NpmRuntimeError> {
    let policy = FetchPolicy::new([FetchDestination::new("https", "registry.npmjs.org")]);
    let budget = DownloadBudget::new(npm_bundle::MAX_BUNDLE_BYTES);
    let workspace =
        crate::workspace::Workspace::open(stage).map_err(|_| NpmRuntimeError::Failed)?;
    for (index, package) in binding.tarballs().iter().enumerate() {
        if cancellation.is_cancelled() {
            return Err(NpmRuntimeError::Cancelled);
        }
        let remaining = MAX_INPUT_TIME.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return Err(NpmRuntimeError::PreparationTimedOut);
        }
        let destination = format!("tarballs/{index:04}.tgz");
        let result = http_download_cancellable(DownloadCall {
            policy: &policy,
            transport,
            workspace: &workspace,
            destination: &destination,
            expected_destination_sha256: None,
            url: package.resolved_url(),
            limits: DownloadLimits {
                max_file_bytes: npm_bundle::MAX_TARBALL_BYTES,
                request_timeout: REQUEST_TIME.min(remaining),
                max_redirect_hops: 5,
            },
            budget: &budget,
            cancellation,
        })
        .await;
        result.map_err(map_download_error)?;
    }
    Ok(())
}

fn map_download_error(error: DownloadError) -> NpmRuntimeError {
    match error {
        DownloadError::Refused(_) | DownloadError::TooManyRedirects { .. } => {
            NpmRuntimeError::UntrustedOrigin
        }
        DownloadError::FileTooLarge { .. } | DownloadError::BudgetExhausted { .. } => {
            NpmRuntimeError::InputTooLarge
        }
        DownloadError::Cancelled => NpmRuntimeError::Cancelled,
        _ => NpmRuntimeError::Failed,
    }
}

fn map_podman_error(error: PodmanError) -> NpmRuntimeError {
    match error {
        PodmanError::Cancelled => NpmRuntimeError::Cancelled,
        PodmanError::CleanupUnknown { .. } => NpmRuntimeError::Unknown,
        _ => NpmRuntimeError::Failed,
    }
}
