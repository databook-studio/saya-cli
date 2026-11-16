//! Settled host process facts for one closed npm check action.

use super::{
    CleanupStatus,
    npm_receipt::{NpmCheckAction, NpmCheckStatus},
};

/// Host-observed facts; script output is bounded, redacted, and may be lossy UTF-8.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NpmCheckOutcome {
    pub(super) source_sha256: String,
    pub(super) snapshot_sha256: String,
    pub(super) manifest_sha256: String,
    pub(super) lock_sha256: String,
    pub(super) input_key_sha256: String,
    pub(super) bundle_sha256: String,
    pub(super) effective_toolchain_id: String,
    pub(super) source_snapshot_policy_id: String,
    pub(super) action: NpmCheckAction,
    pub(super) status: NpmCheckStatus,
    pub(super) client_pid: u32,
    pub(super) exit_code: Option<i32>,
    pub(super) signal: Option<i32>,
    pub(super) joined: bool,
    pub(super) timed_out: bool,
    pub(super) cancelled: bool,
    pub(super) killed_orphans: bool,
    pub(super) elapsed_ms: u64,
    pub(super) stdout: String,
    pub(super) stdout_total_bytes: u64,
    pub(super) stdout_dropped_bytes: u64,
    pub(super) stdout_truncated: bool,
    pub(super) stderr: String,
    pub(super) stderr_total_bytes: u64,
    pub(super) stderr_dropped_bytes: u64,
    pub(super) stderr_truncated: bool,
    pub(super) output_sha256: String,
    pub(super) cleanup: CleanupStatus,
}

impl NpmCheckOutcome {
    pub fn source_sha256(&self) -> &str {
        &self.source_sha256
    }
    pub fn snapshot_sha256(&self) -> &str {
        &self.snapshot_sha256
    }
    pub fn manifest_sha256(&self) -> &str {
        &self.manifest_sha256
    }
    pub fn lock_sha256(&self) -> &str {
        &self.lock_sha256
    }
    pub fn input_key_sha256(&self) -> &str {
        &self.input_key_sha256
    }
    pub fn bundle_sha256(&self) -> &str {
        &self.bundle_sha256
    }
    pub fn effective_toolchain_id(&self) -> &str {
        &self.effective_toolchain_id
    }
    pub fn source_snapshot_policy_id(&self) -> &str {
        &self.source_snapshot_policy_id
    }
    pub fn action(&self) -> NpmCheckAction {
        self.action
    }
    /// A zero host-client exit is a normal completion of the fixed controller.
    pub fn status(&self) -> NpmCheckStatus {
        self.status
    }
    /// PID of the host Podman client process, not the container's Node process.
    pub fn client_pid(&self) -> u32 {
        self.client_pid
    }
    /// Exit code reported by the host Podman client. Nonzero alone does not
    /// prove the npm script started or returned that code.
    pub fn exit_code(&self) -> Option<i32> {
        self.exit_code
    }
    /// Signal reported for the host Podman client process.
    pub fn signal(&self) -> Option<i32> {
        self.signal
    }
    pub fn joined(&self) -> bool {
        self.joined
    }
    pub fn timed_out(&self) -> bool {
        self.timed_out
    }
    pub fn cancelled(&self) -> bool {
        self.cancelled
    }
    pub fn killed_orphans(&self) -> bool {
        self.killed_orphans
    }
    pub fn elapsed_ms(&self) -> u64 {
        self.elapsed_ms
    }
    pub fn stdout(&self) -> &str {
        &self.stdout
    }
    pub fn stdout_total_bytes(&self) -> u64 {
        self.stdout_total_bytes
    }
    pub fn stdout_dropped_bytes(&self) -> u64 {
        self.stdout_dropped_bytes
    }
    pub fn stdout_truncated(&self) -> bool {
        self.stdout_truncated
    }
    pub fn stderr(&self) -> &str {
        &self.stderr
    }
    pub fn stderr_total_bytes(&self) -> u64 {
        self.stderr_total_bytes
    }
    pub fn stderr_dropped_bytes(&self) -> u64 {
        self.stderr_dropped_bytes
    }
    pub fn stderr_truncated(&self) -> bool {
        self.stderr_truncated
    }
    pub fn output_bytes(&self) -> u64 {
        self.stdout_total_bytes
            .saturating_add(self.stderr_total_bytes)
    }
    pub fn output_truncated(&self) -> bool {
        self.stdout_truncated || self.stderr_truncated
    }
    pub fn output_text_may_be_lossy(&self) -> bool {
        true
    }
    pub fn output_sha256(&self) -> &str {
        &self.output_sha256
    }
    pub fn cleanup(&self) -> CleanupStatus {
        self.cleanup
    }
}
