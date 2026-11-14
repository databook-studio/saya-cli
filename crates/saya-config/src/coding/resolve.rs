use std::collections::BTreeMap;

use crate::{ConfigError, JobFile, JobRootMode};

use super::ExtensionProtocol;

/// Effective trusted coding settings. These settings shape admission only; they
/// never grant model, filesystem, process, network, or database authority.
///
/// The run's ceilings and `budget_mode` are resolved beside every other
/// surface's, in [`crate::ResolvedJobLimits`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ResolvedCoding {
    pub extensions: BTreeMap<String, super::ExtensionProfile>,
    /// A validated trusted LSP profile selected for post-edit diagnostics.
    pub automatic_lsp_profile: Option<String>,
    /// The absolute path of the trusted directory the coding host and its
    /// extensions launch programs from. `None` when undeclared: the host falls
    /// back to its own default directory, and a job whose program cannot be
    /// provisioned there fails closed. The resolve never checks existence, so
    /// a dangling path cannot break `saya ask`.
    pub program_dir: Option<std::path::PathBuf>,
    /// Where a job works when its launch states no mode. `in_place` is the
    /// default: the folder itself, with tool edits captured as before-images.
    pub root_mode: JobRootMode,
}

pub(crate) fn resolve(file: &JobFile) -> Result<ResolvedCoding, ConfigError> {
    let program_dir = match file.program_dir.clone() {
        Some(dir) if !dir.is_absolute() => {
            return Err(ConfigError::RelativeCodingProgramDir {
                path: dir.display().to_string(),
            });
        }
        other => other,
    };
    let extensions = super::extensions::resolve(file.extensions.as_ref())?;
    let automatic_lsp_profile = file.automatic_lsp_profile.clone();
    if automatic_lsp_profile.as_ref().is_some_and(|name| {
        extensions
            .get(name)
            .is_none_or(|profile| profile.protocol() != ExtensionProtocol::Lsp)
    }) {
        return Err(ConfigError::InvalidCodingExtension);
    }
    Ok(ResolvedCoding {
        extensions,
        automatic_lsp_profile,
        program_dir,
        root_mode: file.root_mode.unwrap_or_default(),
    })
}
