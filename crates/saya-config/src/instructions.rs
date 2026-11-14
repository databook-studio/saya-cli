use std::path::{Path, PathBuf};

use crate::{ConfigError, model::InstructionsFile};

/// The personal instructions file name, always resolved inside the user
/// config directory. Never a config value: no config layer — especially the
/// untrusted project layer — may redirect where the personal file is read
/// from.
pub const PERSONAL_INSTRUCTIONS_FILE: &str = "instructions.md";

/// The project instructions file name, read from the workspace root.
pub const PROJECT_INSTRUCTIONS_FILE: &str = "AGENTS.md";

/// Ceiling for project instructions: a workspace-root AGENTS.md larger than
/// this is truncated, never enlarged.
pub const DEFAULT_PROJECT_INSTRUCTION_BYTES: usize = 32 * 1024;

/// Ceiling for personal instructions, half the project allowance.
pub const DEFAULT_PERSONAL_INSTRUCTION_BYTES: usize = 16 * 1024;

/// Ceiling on the aggregate instruction context both sources may contribute.
pub const DEFAULT_TOTAL_INSTRUCTION_BYTES: usize = 48 * 1024;

/// Floor on any declared limit. Smaller is a typo, not an intent: a limit
/// this small could not hold a meaningful instruction file.
pub const MIN_INSTRUCTION_BYTES: usize = 256;

/// Effective instruction-source settings, resolved from `[instructions]`
/// plus the frozen-decision defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedInstructions {
    /// Whether project instructions (workspace-root AGENTS.md) load.
    pub project_enabled: bool,
    /// Whether personal instructions (user config directory) load.
    pub personal_enabled: bool,
    /// Per-file byte ceiling for project instructions; truncation is marked,
    /// never enlarged past this.
    pub project_max_bytes: usize,
    /// Per-file byte ceiling for personal instructions.
    pub personal_max_bytes: usize,
    /// Aggregate byte ceiling across both sources.
    pub total_max_bytes: usize,
}

impl Default for ResolvedInstructions {
    fn default() -> Self {
        Self {
            project_enabled: true,
            personal_enabled: true,
            project_max_bytes: DEFAULT_PROJECT_INSTRUCTION_BYTES,
            personal_max_bytes: DEFAULT_PERSONAL_INSTRUCTION_BYTES,
            total_max_bytes: DEFAULT_TOTAL_INSTRUCTION_BYTES,
        }
    }
}

/// Where the personal instructions file lives: one name, one directory, set
/// in code. A config value here would let the project layer (or any layer)
/// point the read at a file of its own choosing.
pub fn personal_instructions_path(user_config_dir: &Path) -> PathBuf {
    user_config_dir.join(PERSONAL_INSTRUCTIONS_FILE)
}

/// Resolve `[instructions]` into concrete values, validating the bounds.
/// Each limit may be lowered but never raised above its default — the
/// defaults are the ceilings, matching the frozen instruction caps.
pub(crate) fn resolve(file: &InstructionsFile) -> Result<ResolvedInstructions, ConfigError> {
    let resolved = ResolvedInstructions {
        project_enabled: file.project.unwrap_or(true),
        personal_enabled: file.personal.unwrap_or(true),
        project_max_bytes: file
            .project_max_bytes
            .unwrap_or(DEFAULT_PROJECT_INSTRUCTION_BYTES),
        personal_max_bytes: file
            .personal_max_bytes
            .unwrap_or(DEFAULT_PERSONAL_INSTRUCTION_BYTES),
        total_max_bytes: file
            .total_max_bytes
            .unwrap_or(DEFAULT_TOTAL_INSTRUCTION_BYTES),
    };
    require_limit(
        "instructions.project_max_bytes",
        resolved.project_max_bytes,
        DEFAULT_PROJECT_INSTRUCTION_BYTES,
    )?;
    require_limit(
        "instructions.personal_max_bytes",
        resolved.personal_max_bytes,
        DEFAULT_PERSONAL_INSTRUCTION_BYTES,
    )?;
    require_limit(
        "instructions.total_max_bytes",
        resolved.total_max_bytes,
        DEFAULT_TOTAL_INSTRUCTION_BYTES,
    )?;
    Ok(resolved)
}

/// Rejects a limit outside `MIN_INSTRUCTION_BYTES..=<its default>`. Both
/// ends are reported — below the floor the limit cannot hold meaningful
/// instructions, and above the default the user would be raising a
/// resource cap that exists to bound untrusted-file reads.
fn require_limit(field: &'static str, value: usize, default: usize) -> Result<(), ConfigError> {
    if (MIN_INSTRUCTION_BYTES..=default).contains(&value) {
        Ok(())
    } else {
        Err(ConfigError::SettingOutOfRange {
            field,
            value,
            min: MIN_INSTRUCTION_BYTES,
            max: default,
        })
    }
}
