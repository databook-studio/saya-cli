//! Settled offline input-validation receipt construction.

use super::{
    CleanupStatus, NODE_IMAGE_MANIFEST_DIGEST, NODE_VERSION, NPM_VERSION,
    npm_bundle::VerifiedBundle, npm_bundle_receipt::NpmInputBundleReceipt,
    npm_lock::NpmInputBinding, npm_receipt::output_sha256, podman::Output,
};

pub(super) fn bundle_receipt(
    binding: &NpmInputBinding,
    bundle: &VerifiedBundle,
    output: Output,
    cleanup: CleanupStatus,
) -> NpmInputBundleReceipt {
    let hash = output_sha256(&output.stdout.text, &output.stderr.text);
    let stdout = output.stdout;
    let stderr = output.stderr;
    NpmInputBundleReceipt {
        input_key_sha256: binding.input_key_sha256().to_owned(),
        bundle_sha256: bundle.bundle_sha256.clone(),
        manifest_sha256: binding.manifest_sha256().to_owned(),
        lock_sha256: binding.lock_sha256().to_owned(),
        image_manifest_digest: NODE_IMAGE_MANIFEST_DIGEST.to_owned(),
        node_version: NODE_VERSION.to_owned(),
        npm_version: NPM_VERSION.to_owned(),
        tarball_count: bundle.tarball_count,
        tarball_bytes: bundle.tarball_bytes,
        validation_client_pid: output.pid,
        validation_exit_code: output.exit_code.unwrap_or(1),
        validation_signal: output.signal,
        validation_elapsed_ms: output.duration_ms,
        validation_killed_orphans: output.killed_orphans,
        validation_stdout: stdout.text,
        validation_stdout_total_bytes: stdout.total_bytes,
        validation_stdout_dropped_bytes: stdout.dropped_bytes,
        validation_stdout_truncated: stdout.truncated,
        validation_stderr: stderr.text,
        validation_stderr_total_bytes: stderr.total_bytes,
        validation_stderr_dropped_bytes: stderr.dropped_bytes,
        validation_stderr_truncated: stderr.truncated,
        validation_output_sha256: hash,
        cleanup,
    }
}
