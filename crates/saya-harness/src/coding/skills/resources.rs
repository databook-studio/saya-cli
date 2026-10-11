//! Explicit resource snapshots. No resource is executed or recursively followed.
use super::{SkillEntry, SkillLoadError, SkillRoot, load_skill_body};

pub const MAX_SKILL_RESOURCES: usize = 16;
pub const MAX_SKILL_RESOURCE_BYTES: usize = 64 * 1024;
pub const MAX_SKILL_TOTAL_RESOURCE_BYTES: usize = 256 * 1024;

/// Inert content from the selected skill's directory. No authority is carried.
#[derive(Clone, PartialEq, Eq)]
pub struct SkillResource {
    reference: String,
    text: String,
    source_sha256: String,
}

impl SkillResource {
    pub fn reference(&self) -> &str {
        &self.reference
    }
    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn source_sha256(&self) -> &str {
        &self.source_sha256
    }
}

impl std::fmt::Debug for SkillResource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SkillResource")
            .field("bytes", &self.text.len())
            .finish_non_exhaustive()
    }
}

/// Loads only explicitly selected relative resource paths. Nested references in
/// returned text remain text; there is no recursive expansion or cycle traversal.
/// Duplicate paths and a reference back to SKILL.md are refused before opening.
pub fn load_skill_resources(
    roots: &[SkillRoot],
    entry: &SkillEntry,
    references: &[String],
) -> Result<Vec<SkillResource>, SkillLoadError> {
    if references.len() > MAX_SKILL_RESOURCES {
        return Err(SkillLoadError::TooManyResources);
    }
    let mut seen = std::collections::HashSet::new();
    for reference in references {
        if !safe_relative(reference) || reference == "SKILL.md" || !seen.insert(reference) {
            return Err(SkillLoadError::InvalidResourceReference);
        }
    }
    // Do not let a fabricated entry escape its selected skill directory.
    if !safe_relative(&entry.relative_dir) || entry.relative_dir.contains('/') {
        return Err(SkillLoadError::InvalidResourceReference);
    }
    let _body = load_skill_body(roots, entry)?;
    #[cfg(not(unix))]
    return Err(SkillLoadError::Unsupported);
    #[cfg(unix)]
    {
        let root = roots
            .get(entry.root_index)
            .ok_or(SkillLoadError::UnknownRoot)?;
        let mut total = 0;
        references
            .iter()
            .map(|reference| {
                let resource = read(root, entry, reference)?;
                total += resource.text.len();
                if total > MAX_SKILL_TOTAL_RESOURCE_BYTES {
                    return Err(SkillLoadError::TooLarge {
                        max: MAX_SKILL_TOTAL_RESOURCE_BYTES,
                    });
                }
                Ok(resource)
            })
            .collect()
    }
}

fn safe_relative(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 1024
        && !path.contains(['\\', ':', '\0'])
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

#[cfg(unix)]
fn read(
    root: &SkillRoot,
    entry: &SkillEntry,
    reference: &str,
) -> Result<SkillResource, SkillLoadError> {
    use sha2::{Digest, Sha256};
    use std::{io::Read, os::unix::fs::MetadataExt};
    let path = format!("{}/{reference}", entry.relative_dir);
    let anchor = root
        .workspace
        .anchor(&path, false)
        .map_err(|_| SkillLoadError::Unreadable)?;
    let stat = anchor.stat().ok_or(SkillLoadError::Missing)?;
    if stat.is_symlink() {
        return Err(SkillLoadError::Symlink);
    }
    if !stat.is_file() {
        return Err(SkillLoadError::NotRegular);
    }
    let file = anchor
        .open_file(libc::O_RDONLY | libc::O_NONBLOCK | libc::O_NOCTTY, 0)
        .map_err(|_| SkillLoadError::Unreadable)?;
    let opened = file.metadata().map_err(|_| SkillLoadError::Unreadable)?;
    if !opened.is_file() || (opened.dev(), opened.ino()) != stat.identity() {
        return Err(SkillLoadError::MetadataChanged);
    }
    let mut bytes = Vec::new();
    file.take(MAX_SKILL_RESOURCE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| SkillLoadError::Unreadable)?;
    if bytes.len() > MAX_SKILL_RESOURCE_BYTES {
        return Err(SkillLoadError::TooLarge {
            max: MAX_SKILL_RESOURCE_BYTES,
        });
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| SkillLoadError::InvalidText)?;
    if text.contains('\0') {
        return Err(SkillLoadError::InvalidText);
    }
    Ok(SkillResource {
        reference: reference.into(),
        text: text.into(),
        source_sha256: format!("{:x}", Sha256::digest(&bytes)),
    })
}
