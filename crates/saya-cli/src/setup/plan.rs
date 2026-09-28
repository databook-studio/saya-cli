//! Turns a draft into a [`SetupPlan`]: a bounded, validated description of
//! the user-level file writes. Planning performs no writes of its own.

use std::path::Path;

use saya_config::{ConfigFile, ConnectionsFile};

use super::{
    CONFIG_FILE, CONNECTIONS_FILE, MAX_FILE_BYTES, SetupError, atomic,
    draft::{ProfileDraft, ProviderDraft, SetupDraft},
    render,
};

/// What a plan intends to write. Contents are final; commit only publishes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupPlan {
    pub writes: Vec<PlannedWrite>,
    pub notes: Vec<String>,
}

/// One planned file write inside the setup directory. `file` is a plain name
/// (`config.toml` / `connections.toml`), never a path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedWrite {
    pub file: String,
    /// True when the file does not exist yet and the commit will create it.
    pub created: bool,
    pub content: String,
}

/// Reads the existing files (bounded) and computes the plan for `draft`.
///
/// - `connections.toml` absent → create with the profile block; present → the
///   profile must not already exist, and the new content keeps the existing
///   bytes as an exact prefix (newline repair, blank line, block).
/// - `config.toml` absent and a provider is drafted → create with only an
///   `[ai]` section; present → never modified, a note carries the snippet.
///
/// Every planned content must parse with its real parser, or the plan fails.
pub fn plan(dir: &Path, draft: &SetupDraft) -> Result<SetupPlan, SetupError> {
    draft.validate()?;
    let mut writes = Vec::new();
    let mut notes = Vec::new();
    if let Some(profile) = &draft.profile {
        plan_connections(dir, profile, &mut writes)?;
    }
    if let Some(provider) = &draft.provider {
        plan_config(dir, provider, &mut writes, &mut notes)?;
    }
    validate_writes(&writes, draft)?;
    Ok(SetupPlan { writes, notes })
}

fn plan_connections(
    dir: &Path,
    profile: &ProfileDraft,
    writes: &mut Vec<PlannedWrite>,
) -> Result<(), SetupError> {
    let path = dir.join(CONNECTIONS_FILE);
    let block = render::profile_block(&profile.name, &profile.profile)?;
    match atomic::read_optional_bounded(&path, MAX_FILE_BYTES)? {
        None => writes.push(PlannedWrite {
            file: CONNECTIONS_FILE.into(),
            created: true,
            content: block,
        }),
        Some(existing_bytes) => {
            let existing = String::from_utf8(existing_bytes).map_err(|_| {
                SetupError::Existing(format!("{} is not valid UTF-8", path.display()))
            })?;
            let parsed = ConnectionsFile::from_toml(&existing).map_err(|error| {
                SetupError::Existing(format!("{} does not parse: {error}", path.display()))
            })?;
            if parsed.profiles.contains_key(&profile.name) {
                return Err(SetupError::ProfileExists(profile.name.clone()));
            }
            let mut content = existing;
            if !content.ends_with('\n') {
                content.push('\n');
            }
            content.push('\n');
            content.push_str(&block);
            writes.push(PlannedWrite {
                file: CONNECTIONS_FILE.into(),
                created: false,
                content,
            });
        }
    }
    Ok(())
}

fn plan_config(
    dir: &Path,
    provider: &ProviderDraft,
    writes: &mut Vec<PlannedWrite>,
    notes: &mut Vec<String>,
) -> Result<(), SetupError> {
    let path = dir.join(CONFIG_FILE);
    let section = render::ai_section(provider)?;
    match atomic::read_optional_bounded(&path, MAX_FILE_BYTES)? {
        None => writes.push(PlannedWrite {
            file: CONFIG_FILE.into(),
            created: true,
            content: section,
        }),
        Some(_) => notes.push(format!(
            "{} already exists; saya did not modify it. To enable the provider, \
             add this section to config.toml:\n\n{section}",
            path.display()
        )),
    }
    Ok(())
}

/// Belt and suspenders: every planned content parses with the real parser it
/// will be read back with, and carries what the draft asked for.
fn validate_writes(writes: &[PlannedWrite], draft: &SetupDraft) -> Result<(), SetupError> {
    for write in writes {
        if write.file == CONNECTIONS_FILE {
            let parsed = ConnectionsFile::from_toml(&write.content).map_err(|error| {
                SetupError::InvalidResult(format!(
                    "planned connections.toml does not parse: {error}"
                ))
            })?;
            if let Some(profile) = &draft.profile
                && !parsed.profiles.contains_key(&profile.name)
            {
                return Err(SetupError::InvalidResult(format!(
                    "planned connections.toml does not carry profile {:?}",
                    profile.name
                )));
            }
        } else if write.file == CONFIG_FILE {
            let parsed = ConfigFile::from_toml(&write.content).map_err(|error| {
                SetupError::InvalidResult(format!("planned config.toml does not parse: {error}"))
            })?;
            if let Some(provider) = &draft.provider
                && parsed.ai.provider != Some(provider.provider)
            {
                return Err(SetupError::InvalidResult(
                    "planned config.toml lost the provider".into(),
                ));
            }
        }
    }
    Ok(())
}
