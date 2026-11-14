use saya_agent::CancellationToken;
use tokio::time::Instant;

#[cfg(unix)]
use crate::coding::git::GitPathStatus;

use super::{SnapshotError, SnapshotFileMode};

pub(super) fn path_string(path: &std::path::Path) -> Result<String, SnapshotError> {
    path.to_str()
        .map(str::to_owned)
        .ok_or(SnapshotError::unsafe_path())
}

/// Git marks a directory record (an ignored directory under
/// `--untracked-files=all`) with a trailing `/`; the snapshot namespace names
/// paths without that marker.
pub(super) fn snapshot_status_path(path: &std::path::Path) -> Result<String, SnapshotError> {
    let path = path_string(path)?;
    Ok(path.strip_suffix('/').unwrap_or(&path).to_owned())
}

#[cfg(unix)]
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
    match mode {
        0o100755 => SnapshotFileMode::Executable,
        0o120000 => SnapshotFileMode::Symlink,
        _ => SnapshotFileMode::Regular,
    }
}

/// One rule in the sensitive-path set.
#[derive(Clone, Copy)]
enum SensitiveRule {
    /// A directory component whose contents are always private.
    Component(&'static str),
    /// A file name that is always private.
    Exact(&'static str),
    /// A file-name prefix (the `.env.*` family).
    Prefix(&'static str),
    /// A file extension, without the dot.
    Extension(&'static str),
}

/// The complete sensitive-path set, shared by snapshot selection and the
/// in-place workspace predicates (delete, search, before-images). Exact
/// credential names, the `.env.*` family, the key/certificate extensions, and
/// the credential-store directories. Names that merely contain `secret` or
/// `credentials` (for example `secrets_manager.py`) and generic `.config`,
/// `.claude`, or `.docker` directories are ordinary repository content and are
/// deliberately absent.
const SENSITIVE_PATHS: &[SensitiveRule] = &[
    SensitiveRule::Component(".aws"),
    SensitiveRule::Component(".ssh"),
    SensitiveRule::Component(".azure"),
    SensitiveRule::Component(".gnupg"),
    SensitiveRule::Component(".kube"),
    SensitiveRule::Component(".codex"),
    SensitiveRule::Component(".saya"),
    SensitiveRule::Exact(".env"),
    SensitiveRule::Exact(".netrc"),
    SensitiveRule::Exact(".npmrc"),
    SensitiveRule::Exact(".pypirc"),
    SensitiveRule::Exact("credentials.json"),
    SensitiveRule::Exact("id_rsa"),
    SensitiveRule::Exact("id_ed25519"),
    SensitiveRule::Prefix(".env."),
    SensitiveRule::Extension("pem"),
    SensitiveRule::Extension("key"),
    SensitiveRule::Extension("p12"),
    SensitiveRule::Extension("pfx"),
];

/// Whether a path holds a credential shape. The one rule table above decides
/// both snapshot omission and the in-place workspace predicates; a tracked
/// path is repository content and is included with a warning instead of being
/// omitted.
pub fn sensitive_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    let name = lower.rsplit('/').next().unwrap_or(&lower);
    SENSITIVE_PATHS.iter().any(|rule| match *rule {
        SensitiveRule::Component(component) => lower.split('/').any(|part| part == component),
        SensitiveRule::Exact(exact) => name == exact,
        SensitiveRule::Prefix(prefix) => name.starts_with(prefix),
        SensitiveRule::Extension(extension) => name
            .rsplit_once('.')
            .is_some_and(|(_, found)| found == extension),
    })
}
