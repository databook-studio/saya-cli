use std::path::Path;

use saya_agent::CancellationToken;
use saya_types::GitObjectId;

use super::{
    command::{GitConfigRisk, GitError, GitOperation, GitRepository, require_success},
    request::GitRequest,
    service::GitService,
};

const MAX_ATTRIBUTE_BYTES: usize = 1024 * 1024;
const MAX_ATTRIBUTE_FILES: usize = 16;
const MAX_ATTRIBUTE_TOTAL_BYTES: u64 = 4 * 1024 * 1024;
const MAX_BASE_FILE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_BASE_TREE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_BASE_TREE_ENTRIES: usize = 20_000;

struct AttributeBlob {
    object: GitObjectId,
    size: u64,
}

pub(super) async fn validate_materialization(
    service: &GitService,
    repository: &GitRepository,
    base: &GitObjectId,
    cancellation: &CancellationToken,
) -> Result<(), GitError> {
    let index = run(
        service,
        repository.root(),
        GitRequest::IndexEntries,
        cancellation,
    )
    .await?;
    if index.stdout.split(|byte| *byte == 0).any(|record| {
        record
            .first()
            .is_some_and(|tag| matches!(*tag, b'S' | b's'))
            && record.get(1) == Some(&b' ')
    }) {
        return Err(GitError::UnsupportedConfiguration(
            GitConfigRisk::SparseCheckout,
        ));
    }
    let tree = run(
        service,
        repository.root(),
        GitRequest::TreeEntries(base),
        cancellation,
    )
    .await?;
    let mut entries = 0usize;
    let mut total_bytes = 0u64;
    let mut attribute_bytes = 0u64;
    let mut attributes = Vec::new();
    for record in tree
        .stdout
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
    {
        entries = entries.saturating_add(1);
        if entries > MAX_BASE_TREE_ENTRIES {
            return Err(GitError::RepositoryTooLarge);
        }
        let tab = record
            .iter()
            .position(|byte| *byte == b'\t')
            .ok_or(GitError::MalformedOutput)?;
        let mut header = record[..tab]
            .split(|byte| byte.is_ascii_whitespace())
            .filter(|field| !field.is_empty());
        let mode = header.next().ok_or(GitError::MalformedOutput)?;
        let kind = header.next().ok_or(GitError::MalformedOutput)?;
        let object = header.next().ok_or(GitError::MalformedOutput)?;
        if mode == b"160000" {
            return Err(GitError::UnsupportedConfiguration(GitConfigRisk::Submodule));
        }
        if mode != b"100644" && mode != b"100755" && mode != b"120000" {
            return Err(GitError::UnsupportedTreeEntry);
        }
        if kind != b"blob" {
            return Err(GitError::UnsupportedTreeEntry);
        }
        let size = header.next().ok_or(GitError::MalformedOutput)?;
        if header.next().is_some() {
            return Err(GitError::MalformedOutput);
        }
        let size = parse_size(size)?;
        if size > MAX_BASE_FILE_BYTES {
            return Err(GitError::RepositoryTooLarge);
        }
        total_bytes = total_bytes
            .checked_add(size)
            .filter(|total| *total <= MAX_BASE_TREE_BYTES)
            .ok_or(GitError::RepositoryTooLarge)?;
        let path = &record[tab + 1..];
        if kind == b"blob"
            && path
                .rsplit(|byte| *byte == b'/')
                .next()
                .is_some_and(|name| name == b".gitattributes")
        {
            if size > MAX_ATTRIBUTE_BYTES as u64 || attributes.len() >= MAX_ATTRIBUTE_FILES {
                return Err(GitError::RepositoryTooLarge);
            }
            attribute_bytes = attribute_bytes
                .checked_add(size)
                .filter(|total| *total <= MAX_ATTRIBUTE_TOTAL_BYTES)
                .ok_or(GitError::RepositoryTooLarge)?;
            let object = parse_object_id(object, repository)?;
            attributes.push(AttributeBlob { object, size });
        }
    }
    for attribute in attributes {
        let output = run(
            service,
            repository.root(),
            GitRequest::ReadBlob(&attribute.object),
            cancellation,
        )
        .await?;
        if output.stdout.len() as u64 != attribute.size {
            return Err(GitError::MalformedOutput);
        }
        let contents = std::str::from_utf8(&output.stdout)
            .map_err(|_| GitError::UnsupportedConfiguration(GitConfigRisk::Filter))?;
        if has_filter_attribute(contents) {
            return Err(GitError::UnsupportedConfiguration(GitConfigRisk::Filter));
        }
    }
    Ok(())
}

fn parse_size(bytes: &[u8]) -> Result<u64, GitError> {
    std::str::from_utf8(bytes)
        .map_err(|_| GitError::MalformedOutput)?
        .parse()
        .map_err(|_| GitError::UnsupportedConfiguration(GitConfigRisk::PartialClone))
}

fn parse_object_id(bytes: &[u8], repository: &GitRepository) -> Result<GitObjectId, GitError> {
    let value = std::str::from_utf8(bytes).map_err(|_| GitError::MalformedOutput)?;
    let object = GitObjectId::parse(value).map_err(|_| GitError::MalformedOutput)?;
    if !super::repository_check::object_id_matches(repository.object_format(), &object) {
        return Err(GitError::MalformedOutput);
    }
    Ok(object)
}

async fn run(
    service: &GitService,
    cwd: &Path,
    request: GitRequest<'_>,
    cancellation: &CancellationToken,
) -> Result<super::process::GitOutput, GitError> {
    let operation = request.operation();
    let output = service.run_request(cwd, request, cancellation).await?;
    require_success(output, operation)
}

fn has_filter_attribute(attributes: &str) -> bool {
    attributes.lines().any(|line| {
        let line = line.trim_start();
        if line.is_empty() || line.starts_with('#') {
            return false;
        }
        line.split_ascii_whitespace()
            .skip(1)
            .any(|attribute| attribute == "filter" || attribute.starts_with("filter="))
    })
}
