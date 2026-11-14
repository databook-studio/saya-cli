//! Closed, offline checks over a bounded source snapshot and verified inputs.

use std::time::Duration;
use std::{fs, path::Path};

use tokio::time::Instant;

use super::{
    NpmCheckRequest, NpmRuntimeError,
    npm::{ensure_ready_registry, open_ready_registry, registry_error},
    npm_bundle, npm_lock,
    npm_receipt::{NpmCheckAction, NpmCheckOutcome, NpmCheckStatus, output_sha256},
    npm_snapshot,
    podman::{NodeAction, NpmActionRequest, Output, Podman, PodmanError},
    podman_process::ticket,
    registry::Registry,
};

const MAX_CHECK_TIME: Duration = Duration::from_secs(600);

pub(super) async fn run(request: NpmCheckRequest<'_>) -> Result<NpmCheckOutcome, NpmRuntimeError> {
    validate_request(&request)?;
    let binding = npm_lock::bind_project_inputs(request.project)?;
    let registry = open_ready_registry(request.state_root)?;
    let mut podman = Podman::discover().map_err(|_| NpmRuntimeError::Failed)?;
    podman.bind_authfile(registry.authfile());
    run_with(registry, podman, binding, request).await
}

pub(super) async fn run_with(
    registry: Registry,
    mut podman: Podman,
    binding: npm_lock::NpmInputBinding,
    request: NpmCheckRequest<'_>,
) -> Result<NpmCheckOutcome, NpmRuntimeError> {
    validate_request(&request)?;
    ensure_ready_registry(&registry)?;
    npm_bundle::cleanup_abandoned_stages(&registry)?;
    let bundle = npm_bundle::load_verified(&registry, &binding)?
        .ok_or(NpmRuntimeError::InputBundleMissing)?;
    let budget = request.timeout.min(MAX_CHECK_TIME);
    let started = Instant::now();
    let operation_ticket = ticket().map_err(|_| NpmRuntimeError::Failed)?;
    let stage = registry
        .create_npm_stage(&operation_ticket)
        .map_err(registry_error)?;
    let source_path = stage.join("source");
    let source = match npm_snapshot::copy_source(
        request.project,
        request.source,
        &source_path,
        request.source_sha256,
        request.cancellation,
        budget,
    ) {
        Ok(source) => source,
        Err(error) => {
            npm_bundle::cleanup_stage(&registry, &stage)?;
            return Err(error);
        }
    };
    if let Err(error) = verify_package_inputs(&source.path, &binding) {
        npm_bundle::cleanup_stage(&registry, &stage)?;
        return Err(error);
    }
    let controller = match npm_bundle::write_controller(&stage) {
        Ok(controller) => controller,
        Err(error) => {
            npm_bundle::cleanup_stage(&registry, &stage)?;
            return Err(error);
        }
    };
    podman.bind_authfile(registry.authfile());
    let remaining = budget.saturating_sub(started.elapsed());
    if remaining.is_zero() {
        npm_bundle::cleanup_stage(&registry, &stage)?;
        return Err(NpmRuntimeError::PreparationTimedOut);
    }
    if request.cancellation.is_cancelled() {
        npm_bundle::cleanup_stage(&registry, &stage)?;
        return Err(NpmRuntimeError::Cancelled);
    }
    if let Err(error) = registry.begin_probe(&operation_ticket) {
        npm_bundle::cleanup_stage(&registry, &stage)?;
        return Err(registry_error(error));
    }
    let action = match request.action {
        NpmCheckAction::TestCi => NodeAction::TestCi,
        NpmCheckAction::Build => NodeAction::Build,
    };
    let result = podman
        .run_npm_action(
            &registry,
            NpmActionRequest {
                ticket: &operation_ticket,
                bundle: &bundle.path,
                controller: &controller,
                source: Some(&source.path),
                action,
                timeout: remaining,
                cancellation: request.cancellation,
            },
        )
        .await;
    let (output, cleanup) = match result {
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
    npm_bundle::cleanup_stage(&registry, &stage).map_err(|_| NpmRuntimeError::Unknown)?;
    podman
        .finish_npm_ticket(&registry, &operation_ticket)
        .map_err(|_| NpmRuntimeError::Unknown)?;
    Ok(check_outcome(
        request.source_sha256,
        &source.sha256,
        &binding,
        &bundle,
        request.action,
        output,
        cleanup,
    ))
}

fn validate_request(request: &NpmCheckRequest<'_>) -> Result<(), NpmRuntimeError> {
    if request.cancellation.is_cancelled() {
        return Err(NpmRuntimeError::Cancelled);
    }
    npm_lock::validate_mount_path(request.state_root)?;
    npm_lock::validate_mount_path(request.project.root())?;
    if !valid_digest(request.source_sha256) || request.timeout.is_zero() {
        return Err(NpmRuntimeError::InvalidProject);
    }
    Ok(())
}

fn verify_package_inputs(
    snapshot: &Path,
    binding: &npm_lock::NpmInputBinding,
) -> Result<(), NpmRuntimeError> {
    for (name, expected) in [
        ("package.json", binding.manifest_bytes()),
        ("package-lock.json", binding.lock_bytes()),
    ] {
        let actual = fs::read(snapshot.join(name)).map_err(|_| NpmRuntimeError::SourceChanged)?;
        if actual.as_slice() != expected {
            return Err(NpmRuntimeError::SourceChanged);
        }
    }
    Ok(())
}

fn check_outcome(
    source_sha256: &str,
    snapshot_sha256: &str,
    binding: &npm_lock::NpmInputBinding,
    bundle: &npm_bundle::VerifiedBundle,
    action: NpmCheckAction,
    output: Output,
    cleanup: super::CleanupStatus,
) -> NpmCheckOutcome {
    let status = if output.cancelled {
        NpmCheckStatus::Cancelled
    } else if output.timed_out {
        NpmCheckStatus::TimedOut
    } else if output.exit_code == Some(0) {
        NpmCheckStatus::Succeeded
    } else {
        NpmCheckStatus::Failed
    };
    let digest = output_sha256(&output.stdout.text, &output.stderr.text);
    NpmCheckOutcome {
        source_sha256: source_sha256.to_owned(),
        snapshot_sha256: snapshot_sha256.to_owned(),
        manifest_sha256: binding.manifest_sha256().to_owned(),
        lock_sha256: binding.lock_sha256().to_owned(),
        input_key_sha256: binding.input_key_sha256().to_owned(),
        bundle_sha256: bundle.bundle_sha256.clone(),
        effective_toolchain_id: npm_bundle::effective_toolchain_id(
            binding.input_key_sha256(),
            &bundle.bundle_sha256,
        ),
        source_snapshot_policy_id: npm_snapshot::SOURCE_SNAPSHOT_POLICY_ID.to_owned(),
        action,
        status,
        client_pid: output.pid,
        exit_code: output.exit_code,
        signal: output.signal,
        joined: true,
        timed_out: output.timed_out,
        cancelled: output.cancelled,
        killed_orphans: output.killed_orphans,
        elapsed_ms: output.duration_ms,
        stdout: output.stdout.text,
        stdout_total_bytes: output.stdout.total_bytes,
        stdout_dropped_bytes: output.stdout.dropped_bytes,
        stdout_truncated: output.stdout.truncated,
        stderr: output.stderr.text,
        stderr_total_bytes: output.stderr.total_bytes,
        stderr_dropped_bytes: output.stderr.dropped_bytes,
        stderr_truncated: output.stderr.truncated,
        output_sha256: digest,
        cleanup,
    }
}

fn map_podman_error(error: PodmanError) -> NpmRuntimeError {
    match error {
        PodmanError::Cancelled => NpmRuntimeError::Cancelled,
        PodmanError::CleanupUnknown { .. } => NpmRuntimeError::Unknown,
        _ => NpmRuntimeError::Failed,
    }
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}
