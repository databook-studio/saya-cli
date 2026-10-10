#[cfg(unix)]
use std::collections::{BTreeMap, BTreeSet};

#[cfg(unix)]
use sha2::{Digest, Sha256};

#[cfg(unix)]
use crate::{HarnessError, workspace::EntryKind};

use super::MAX_SKILL_ROOTS;
#[cfg(unix)]
use super::frontmatter::parse_frontmatter;
#[cfg(unix)]
use super::header::{self, HeaderReadError};
#[cfg(unix)]
use super::metadata::SkillEntry;
use super::metadata::{SkillIndex, SkillIndexError, SkillOrigin, SkillRoot};
#[cfg(unix)]
use super::{MAX_SKILL_ENTRIES, MAX_SKILL_HEADER_BYTES, MAX_SKILL_TOTAL_HEADER_BYTES};

/// Enumerates at most one configured User and Project root and parses only
/// bounded SKILL.md frontmatter. Project metadata wins cross-origin name
/// collisions, independent of caller root ordering.
pub fn discover_skills(roots: &[SkillRoot]) -> Result<SkillIndex, SkillIndexError> {
    if roots.len() > MAX_SKILL_ROOTS {
        return Err(SkillIndexError::TooManyRoots);
    }
    let mut origins = [false; 2];
    for root in roots {
        let origin_index = if root.origin == SkillOrigin::Project {
            1
        } else {
            0
        };
        if origins[origin_index] {
            return Err(SkillIndexError::DuplicateOrigin);
        }
        origins[origin_index] = true;
    }
    #[cfg(not(unix))]
    return Err(SkillIndexError::UnsupportedPlatform);
    #[cfg(unix)]
    discover_unix(roots)
}

#[cfg(unix)]
fn discover_unix(roots: &[SkillRoot]) -> Result<SkillIndex, SkillIndexError> {
    let mut winners = BTreeMap::<String, SkillEntry>::new();
    let mut names_by_origin = [BTreeSet::new(), BTreeSet::new()];
    let mut enumerated = 0usize;
    let mut header_bytes = 0usize;
    for (root_index, root) in roots.iter().enumerate() {
        let remaining = MAX_SKILL_ENTRIES.saturating_sub(enumerated);
        let mut entries = root
            .workspace
            .list("", remaining)
            .map_err(|error| match error {
                HarnessError::BoundsExceeded { .. } => SkillIndexError::EntryLimit {
                    max: MAX_SKILL_ENTRIES,
                },
                _ => SkillIndexError::WorkspaceRefused,
            })?;
        enumerated = enumerated
            .checked_add(entries.len())
            .ok_or(SkillIndexError::EntryLimit {
                max: MAX_SKILL_ENTRIES,
            })?;
        if enumerated > MAX_SKILL_ENTRIES {
            return Err(SkillIndexError::EntryLimit {
                max: MAX_SKILL_ENTRIES,
            });
        }
        entries.sort_by(|left, right| left.name.cmp(&right.name));
        for entry in entries {
            match entry.kind {
                EntryKind::File | EntryKind::Other => continue,
                EntryKind::Symlink => return Err(SkillIndexError::SymlinkRefused),
                EntryKind::Dir => {}
            }
            if !valid_directory_name(&entry.name) {
                return Err(SkillIndexError::InvalidSkillPath);
            }
            let relative_path = format!("{}/SKILL.md", entry.name);
            let remaining_header = MAX_SKILL_TOTAL_HEADER_BYTES
                .checked_sub(header_bytes)
                .ok_or(SkillIndexError::AggregateHeaderTooLarge {
                    max: MAX_SKILL_TOTAL_HEADER_BYTES,
                })?;
            let read_limit = MAX_SKILL_HEADER_BYTES.min(remaining_header);
            let bytes = match header::read(&root.workspace, &relative_path, read_limit) {
                Ok(bytes) => bytes,
                Err(HeaderReadError::Missing) => continue,
                Err(HeaderReadError::TooLarge) if remaining_header < MAX_SKILL_HEADER_BYTES => {
                    return Err(SkillIndexError::AggregateHeaderTooLarge {
                        max: MAX_SKILL_TOTAL_HEADER_BYTES,
                    });
                }
                Err(HeaderReadError::TooLarge) => {
                    return Err(SkillIndexError::HeaderTooLarge {
                        max: MAX_SKILL_HEADER_BYTES,
                    });
                }
                Err(HeaderReadError::Symlink) => return Err(SkillIndexError::SymlinkRefused),
                Err(HeaderReadError::NotRegular) => return Err(SkillIndexError::NotRegularFile),
                Err(HeaderReadError::Unterminated) => {
                    return Err(SkillIndexError::MalformedFrontmatter);
                }
                Err(HeaderReadError::Refused | HeaderReadError::Unreadable) => {
                    return Err(SkillIndexError::WorkspaceRefused);
                }
            };
            header_bytes = header_bytes.checked_add(bytes.len()).ok_or(
                SkillIndexError::AggregateHeaderTooLarge {
                    max: MAX_SKILL_TOTAL_HEADER_BYTES,
                },
            )?;
            if header_bytes > MAX_SKILL_TOTAL_HEADER_BYTES {
                return Err(SkillIndexError::AggregateHeaderTooLarge {
                    max: MAX_SKILL_TOTAL_HEADER_BYTES,
                });
            }
            let parsed = parse_frontmatter(&bytes)?;
            let origin_index = if root.origin == SkillOrigin::Project {
                1
            } else {
                0
            };
            if !names_by_origin[origin_index].insert(parsed.name.clone()) {
                return Err(SkillIndexError::Conflict);
            }
            let digest = Sha256::digest(&bytes);
            let metadata_sha256 = digest.iter().map(|byte| format!("{byte:02x}")).collect();
            let name = parsed.name;
            let candidate = SkillEntry {
                name: name.clone(),
                description: parsed.description,
                origin: root.origin,
                trust_label: root.trust_label.clone(),
                root_index,
                relative_dir: entry.name,
                metadata_sha256,
            };
            let existing_origin = winners.get(&name).map(|existing| existing.origin);
            match existing_origin {
                Some(SkillOrigin::Project) => {}
                Some(SkillOrigin::User) if candidate.origin == SkillOrigin::Project => {
                    winners.insert(name, candidate);
                }
                Some(SkillOrigin::User) => {}
                None => {
                    winners.insert(name, candidate);
                }
            }
        }
    }
    Ok(SkillIndex {
        skills: winners.into_values().collect(),
    })
}

fn valid_directory_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && name.len() <= 255
        && !name
            .chars()
            .any(|character| character.is_control() || matches!(character, '/' | '\\' | '\0'))
}
