use std::path::{Path, PathBuf};

use saya_agent::CancellationToken;
use saya_types::GitObjectId;

use super::{
    command::{
        GitConfigRisk, GitError, GitObjectFormat, GitOperation, GitRepository, require_success,
    },
    request::GitRequest,
    service::GitService,
};

const MAX_LOCAL_CONFIG_BYTES: usize = 1024 * 1024;

pub(super) async fn inspect(
    service: &GitService,
    path: &Path,
    allow_managed: bool,
    cancellation: &CancellationToken,
) -> Result<GitRepository, GitError> {
    let input = std::fs::canonicalize(path).map_err(|_| GitError::NotRepository)?;
    if !std::fs::metadata(&input)
        .map_err(|_| GitError::NotRepository)?
        .is_dir()
    {
        return Err(GitError::NotRepository);
    }
    if !allow_managed && paths_overlap(&input, &service.environment.root) {
        return Err(GitError::NotRepository);
    }
    let bare = run(service, &input, GitRequest::IsBare, cancellation).await?;
    match single_line(&bare.stdout)?.as_str() {
        "true" => return Err(GitError::BareRepository),
        "false" => {}
        _ => return Err(GitError::MalformedOutput),
    }
    let root = run(service, &input, GitRequest::RepositoryRoot, cancellation).await?;
    let root = canonical_line_path(&root.stdout)?;
    if !input.starts_with(&root)
        || (!allow_managed && paths_overlap(&root, &service.environment.root))
    {
        return Err(GitError::NotRepository);
    }
    let common_dir = run(service, &root, GitRequest::CommonDir, cancellation).await?;
    let common_dir = PathBuf::from(single_line(&common_dir.stdout)?);
    let common_dir = if common_dir.is_absolute() {
        common_dir
    } else {
        root.join(common_dir)
    };
    let common_dir = std::fs::canonicalize(common_dir).map_err(|_| GitError::MalformedOutput)?;
    if !std::fs::metadata(&common_dir)
        .map_err(|_| GitError::MalformedOutput)?
        .is_dir()
        || (!allow_managed && paths_overlap(&common_dir, &service.environment.root))
    {
        return Err(GitError::NotRepository);
    }
    let object_format = run(service, &root, GitRequest::ObjectFormat, cancellation).await?;
    let object_format = match single_line(&object_format.stdout)?.as_str() {
        "sha1" => GitObjectFormat::Sha1,
        "sha256" => GitObjectFormat::Sha256,
        _ => return Err(GitError::UnsupportedObjectFormat),
    };
    let config = run(service, &root, GitRequest::LocalConfig, cancellation).await?;
    validate_local_config(&config.stdout)?;
    Ok(GitRepository {
        root,
        common_dir,
        object_format,
    })
}

pub(super) async fn verify(
    service: &GitService,
    repository: &GitRepository,
    cancellation: &CancellationToken,
) -> Result<(), GitError> {
    let current = inspect(service, repository.root(), false, cancellation).await?;
    if &current != repository {
        return Err(GitError::RepositoryChanged);
    }
    Ok(())
}

fn validate_local_config(bytes: &[u8]) -> Result<(), GitError> {
    if bytes.len() > MAX_LOCAL_CONFIG_BYTES {
        return Err(GitError::OutputLimitExceeded);
    }
    for record in bytes
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
    {
        let separator = record
            .iter()
            .position(|byte| *byte == b'\n')
            .ok_or(GitError::MalformedOutput)?;
        let key = std::str::from_utf8(&record[..separator])
            .map_err(|_| GitError::MalformedOutput)?
            .to_ascii_lowercase();
        let value = std::str::from_utf8(&record[separator + 1..])
            .map_err(|_| GitError::MalformedOutput)?
            .trim()
            .to_ascii_lowercase();
        let risk = if key.starts_with("include.") || key.starts_with("includeif.") {
            Some(GitConfigRisk::Include)
        } else if key.starts_with("filter.") {
            Some(GitConfigRisk::Filter)
        } else if key == "extensions.partialclone" {
            Some(GitConfigRisk::PartialClone)
        } else if key.starts_with("remote.") && key.ends_with(".promisor") && truthy(&value) {
            Some(GitConfigRisk::PromisorRemote)
        } else if (key == "core.sparsecheckout" || key == "index.sparse") && truthy(&value) {
            Some(GitConfigRisk::SparseCheckout)
        } else if key == "extensions.worktreeconfig" && truthy(&value) {
            Some(GitConfigRisk::WorktreeConfig)
        } else if key.starts_with("submodule.") && key.ends_with(".recurse") && truthy(&value) {
            Some(GitConfigRisk::SubmoduleRecursion)
        } else {
            None
        };
        if let Some(risk) = risk {
            return Err(GitError::UnsupportedConfiguration(risk));
        }
    }
    Ok(())
}

fn truthy(value: &str) -> bool {
    !matches!(value, "false" | "no" | "off" | "0")
}

pub(super) fn object_id_matches(format: GitObjectFormat, object: &GitObjectId) -> bool {
    match format {
        GitObjectFormat::Sha1 => object.as_str().len() == 40,
        GitObjectFormat::Sha256 => object.as_str().len() == 64,
    }
}

pub(super) fn single_line(bytes: &[u8]) -> Result<String, GitError> {
    let bytes = bytes.strip_suffix(b"\n").ok_or(GitError::MalformedOutput)?;
    if bytes.contains(&b'\n') || bytes.contains(&0) {
        return Err(GitError::MalformedOutput);
    }
    String::from_utf8(bytes.to_vec()).map_err(|_| GitError::MalformedOutput)
}

fn canonical_line_path(bytes: &[u8]) -> Result<PathBuf, GitError> {
    let path = PathBuf::from(single_line(bytes)?);
    if !path.is_absolute() {
        return Err(GitError::MalformedOutput);
    }
    std::fs::canonicalize(path).map_err(|_| GitError::MalformedOutput)
}

fn paths_overlap(left: &Path, right: &Path) -> bool {
    left.starts_with(right) || right.starts_with(left)
}
