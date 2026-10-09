use super::*;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputFile {
    pub format: Option<OutputFormat>,
    pub color: Option<ColorChoice>,
}

/// The `[ui]` section: presentation settings that affect how the TUI paints,
/// not what it does. `theme` selects the colour palette.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiFile {
    pub theme: Option<ThemeChoice>,
}

/// The `[memory]` section.
///
/// Uses `#[serde(deny_unknown_fields)]` so obsolete multi-axis configurations
/// (such as `recall` or `learning`) fail loudly at parse time instead of silently
/// falling back to defaults.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryFile {
    pub mode: Option<MemoryMode>,
    pub max_contracts: Option<u32>,
    pub max_claims_per_contract: Option<u32>,
    pub max_context_bytes: Option<u32>,
}

/// The `[instructions]` section: which instruction sources load and their
/// byte limits. Enablement and limits are protected settings — the project
/// layer's declaration is reverted (see `layers.rs`) — and every limit is
/// validated at resolve time: lowerable, never raisable above its default.
/// There is deliberately no path key: where the personal file is read from
/// is set in code (`personal_instructions_path`), never config, so no layer
/// can redirect the read.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstructionsFile {
    /// Load project instructions (workspace-root AGENTS.md).
    pub project: Option<bool>,
    /// Load personal instructions (user config directory).
    pub personal: Option<bool>,
    /// Per-file byte ceiling for project instructions.
    pub project_max_bytes: Option<usize>,
    /// Per-file byte ceiling for personal instructions.
    pub personal_max_bytes: Option<usize>,
    /// Aggregate byte ceiling across both sources.
    pub total_max_bytes: Option<usize>,
}
