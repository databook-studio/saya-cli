use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Where a coding job works: directly in the stated folder (`in_place`, the
/// default) or in a managed checkout copied from a Git repository.
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JobRootMode {
    /// Edit the stated folder itself; tool edits are captured as before-images.
    #[default]
    InPlace,
    /// Copy the repository into a managed checkout and apply the result back.
    Managed,
}

/// Raw trusted `[job]` settings — the job host's own. Every field stays
/// optional until layers merge, so a higher-precedence layer can override one
/// setting at a time.
///
/// The run's ceilings and `budget_mode` are deliberately *not* here: they live
/// in `[limits.job]`, beside every other surface's ceilings.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct JobFile {
    #[serde(default)]
    pub extensions: Option<BTreeMap<String, crate::coding::ExtensionProfile>>,
    /// Trusted default diagnostic profile. This selects only a named profile;
    /// child runner authority remains an independent requirement.
    pub automatic_lsp_profile: Option<String>,
    /// The absolute path of the trusted directory the job host and its
    /// extensions launch programs from. The directory is operator-owned and
    /// staged before a job: the host only ever reads it. It must be absolute —
    /// the canonical form must not depend on the working directory the config
    /// was loaded from — and existence is deliberately not checked at resolve
    /// time, so a dangling path cannot break `saya ask`; a job that cannot
    /// provision it fails closed instead. The whole `[job]` table is on the
    /// protected list, so an untrusted project layer cannot retarget it.
    pub program_dir: Option<std::path::PathBuf>,
    /// Where a job works when the launch states no flag: the folder itself
    /// (`in_place`, the default) or a managed checkout (`managed`).
    pub root_mode: Option<JobRootMode>,
}
