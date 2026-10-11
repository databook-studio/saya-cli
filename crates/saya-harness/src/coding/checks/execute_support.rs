use super::{CheckExecutionError, CheckExecutionInput, identity::CheckLineage, valid_reference};
use crate::coding::commands::{CommandOutputStatus, CommandSession};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

pub(super) async fn verify_candidate_still_owned(
    input: &CheckExecutionInput<'_>,
    original: &CheckLineage,
) -> Result<(), CheckExecutionError> {
    match original.verify(input.ownership, input.cancellation).await {
        Err(CheckExecutionError::StaleReceipt) => return Err(CheckExecutionError::SourceChanged),
        result => {
            result?;
        }
    }
    Ok(())
}

pub(super) enum SettledOutput {
    Ready(CommandOutputStatus),
    Missing,
    Timeout,
}

pub(super) async fn wait_for_output_status(
    session: &CommandSession,
    timeout: Duration,
) -> SettledOutput {
    let wait = timeout.saturating_add(Duration::from_secs(2));
    match tokio::time::timeout(wait, async {
        loop {
            if let Some(status) = session.output_status() {
                return Some(status);
            }
            if session.is_finished() {
                return None;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    {
        Ok(Some(status)) => SettledOutput::Ready(status),
        Ok(None) => SettledOutput::Missing,
        Err(_) => SettledOutput::Timeout,
    }
}

pub(super) fn external_output_root(
    path: &Path,
    candidate: &Path,
) -> Result<PathBuf, CheckExecutionError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| CheckExecutionError::OutputRoot)?;
    if !metadata.is_dir() {
        return Err(CheckExecutionError::OutputRoot);
    }
    let output = fs::canonicalize(path).map_err(|_| CheckExecutionError::OutputRoot)?;
    let candidate = fs::canonicalize(candidate).map_err(|_| CheckExecutionError::OutputRoot)?;
    if output.as_path() != path || output.starts_with(candidate) {
        return Err(CheckExecutionError::OutputRoot);
    }
    Ok(output)
}

pub(super) fn output_directory_id(
    directory: &Path,
    output_root: &Path,
) -> Result<String, CheckExecutionError> {
    let canonical =
        fs::canonicalize(directory).map_err(|_| CheckExecutionError::OutputUnavailable)?;
    let parent = canonical
        .parent()
        .ok_or(CheckExecutionError::OutputUnavailable)?;
    if parent.file_name().and_then(|name| name.to_str()) != Some("command-output")
        || parent.parent().is_none_or(|root| root != output_root)
    {
        return Err(CheckExecutionError::OutputUnavailable);
    }
    let id = canonical
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(CheckExecutionError::OutputUnavailable)?;
    if !valid_reference(id) {
        return Err(CheckExecutionError::OutputUnavailable);
    }
    Ok(id.to_owned())
}

pub(super) fn digest_argv(
    argv: &[String],
) -> Result<saya_types::Sha256Digest, CheckExecutionError> {
    let bytes = serde_json::to_vec(argv).map_err(|_| CheckExecutionError::InvalidInvocation)?;
    let digest = Sha256::digest(bytes);
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        encoded.push(char::from(b"0123456789abcdef"[(byte >> 4) as usize]));
        encoded.push(char::from(b"0123456789abcdef"[(byte & 0x0f) as usize]));
    }
    saya_types::Sha256Digest::parse(&encoded).map_err(|_| CheckExecutionError::InvalidInvocation)
}
