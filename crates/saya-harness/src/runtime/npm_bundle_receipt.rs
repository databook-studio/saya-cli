//! Host-verified tarball bundle and joined offline-validation facts.

use super::CleanupStatus;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NpmInputBundleReceipt {
    pub(super) input_key_sha256: String,
    pub(super) bundle_sha256: String,
    pub(super) manifest_sha256: String,
    pub(super) lock_sha256: String,
    pub(super) image_manifest_digest: String,
    pub(super) node_version: String,
    pub(super) npm_version: String,
    pub(super) tarball_count: usize,
    pub(super) tarball_bytes: u64,
    pub(super) validation_client_pid: u32,
    pub(super) validation_exit_code: i32,
    pub(super) validation_signal: Option<i32>,
    pub(super) validation_elapsed_ms: u64,
    pub(super) validation_killed_orphans: bool,
    pub(super) validation_stdout: String,
    pub(super) validation_stdout_total_bytes: u64,
    pub(super) validation_stdout_dropped_bytes: u64,
    pub(super) validation_stdout_truncated: bool,
    pub(super) validation_stderr: String,
    pub(super) validation_stderr_total_bytes: u64,
    pub(super) validation_stderr_dropped_bytes: u64,
    pub(super) validation_stderr_truncated: bool,
    pub(super) validation_output_sha256: String,
    pub(super) cleanup: CleanupStatus,
}

impl NpmInputBundleReceipt {
    pub fn input_key_sha256(&self) -> &str {
        &self.input_key_sha256
    }
    pub fn bundle_sha256(&self) -> &str {
        &self.bundle_sha256
    }
    pub fn manifest_sha256(&self) -> &str {
        &self.manifest_sha256
    }
    pub fn lock_sha256(&self) -> &str {
        &self.lock_sha256
    }
    pub fn image_manifest_digest(&self) -> &str {
        &self.image_manifest_digest
    }
    pub fn node_version(&self) -> &str {
        &self.node_version
    }
    pub fn npm_version(&self) -> &str {
        &self.npm_version
    }
    pub fn tarball_count(&self) -> usize {
        self.tarball_count
    }
    pub fn tarball_bytes(&self) -> u64 {
        self.tarball_bytes
    }
    /// PID of the host Podman client process, not the container's Node process.
    pub fn validation_client_pid(&self) -> u32 {
        self.validation_client_pid
    }
    /// Exit code reported by the host Podman client. Nonzero alone does not
    /// prove the npm validation script started or returned that code.
    /// Zero is normal completion of the fixed dependency-validation controller.
    pub fn validation_exit_code(&self) -> i32 {
        self.validation_exit_code
    }
    /// Signal reported for the host Podman client process.
    pub fn validation_signal(&self) -> Option<i32> {
        self.validation_signal
    }
    pub fn validation_elapsed_ms(&self) -> u64 {
        self.validation_elapsed_ms
    }
    pub fn validation_killed_orphans(&self) -> bool {
        self.validation_killed_orphans
    }
    pub fn validation_stdout(&self) -> &str {
        &self.validation_stdout
    }
    pub fn validation_stdout_total_bytes(&self) -> u64 {
        self.validation_stdout_total_bytes
    }
    pub fn validation_stdout_dropped_bytes(&self) -> u64 {
        self.validation_stdout_dropped_bytes
    }
    pub fn validation_stdout_truncated(&self) -> bool {
        self.validation_stdout_truncated
    }
    pub fn validation_stderr(&self) -> &str {
        &self.validation_stderr
    }
    pub fn validation_stderr_total_bytes(&self) -> u64 {
        self.validation_stderr_total_bytes
    }
    pub fn validation_stderr_dropped_bytes(&self) -> u64 {
        self.validation_stderr_dropped_bytes
    }
    pub fn validation_stderr_truncated(&self) -> bool {
        self.validation_stderr_truncated
    }
    pub fn validation_output_sha256(&self) -> &str {
        &self.validation_output_sha256
    }
    pub fn validation_output_bytes(&self) -> u64 {
        self.validation_stdout_total_bytes
            .saturating_add(self.validation_stderr_total_bytes)
    }
    pub fn cleanup(&self) -> CleanupStatus {
        self.cleanup
    }
    pub fn joined(&self) -> bool {
        true
    }

    /// Digest identity includes the verified, content-addressed tarball manifest.
    pub fn effective_toolchain_id(&self) -> String {
        super::npm_bundle::effective_toolchain_id(&self.input_key_sha256, &self.bundle_sha256)
    }
}
