use saya_agent::CancellationToken;
use tokio::time::Instant;

use crate::coding::git::GitPathStatus;

use super::{SnapshotError, SnapshotFileMode};

pub(super) fn path_string(path: &std::path::Path) -> Result<String, SnapshotError> {
    path.to_str()
        .map(str::to_owned)
        .ok_or(SnapshotError::UnsafePath)
}

/// Git marks a directory record (an ignored directory under
/// `--untracked-files=all`) with a trailing `/`; the snapshot namespace names
/// paths without that marker.
pub(super) fn snapshot_status_path(path: &std::path::Path) -> Result<String, SnapshotError> {
    let path = path_string(path)?;
    Ok(path.strip_suffix('/').unwrap_or(&path).to_owned())
}

pub(super) fn status_proves_deletion(status: &[GitPathStatus], path: &str) -> bool {
    let path = std::path::Path::new(path);
    status
        .iter()
        .any(|fact| fact.path() == path && (fact.staged_deleted() || fact.unstaged_deleted()))
}

pub(super) fn check_control(
    cancellation: &CancellationToken,
    deadline: Instant,
) -> Result<(), SnapshotError> {
    if cancellation.is_cancelled() {
        return Err(SnapshotError::Cancelled);
    }
    if Instant::now() >= deadline {
        return Err(SnapshotError::DeadlineExceeded);
    }
    Ok(())
}

pub(super) fn file_mode(mode: u32) -> SnapshotFileMode {
    if mode == 0o100755 {
        SnapshotFileMode::Executable
    } else {
        SnapshotFileMode::Regular
    }
}

pub(super) fn sensitive_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let name = lower.rsplit('/').next().unwrap_or(&lower);
    if lower.split('/').any(|component| {
        matches!(
            component,
            ".aws"
                | ".ssh"
                | ".azure"
                | ".gnupg"
                | ".kube"
                | ".docker"
                | ".config"
                | ".codex"
                | ".claude"
                | ".saya"
        )
    }) {
        return true;
    }
    name == ".env"
        || name.starts_with(".env.")
        || name.ends_with(".pem")
        || name.ends_with(".key")
        || name.ends_with(".p12")
        || name.ends_with(".pfx")
        || name == "id_rsa"
        || name == "id_ed25519"
        || name.contains("credentials")
        || name.contains("secret")
        || name.ends_with(".kdbx")
}
