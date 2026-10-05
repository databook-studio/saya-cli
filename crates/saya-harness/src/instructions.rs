//! The bounded instruction loader: at most two named files — the
//! workspace-root `AGENTS.md` and a personal `instructions.md` inside an
//! explicitly configured directory — read through the contained, no-follow
//! [`Workspace::read`]. No discovery walk, no import handling, no command
//! execution: bodies return literally, as context data that grants nothing.

use std::path::Path;

use self::read::read_one;

mod read;

/// The workspace-root instructions file: only this exact root name is read.
pub const PROJECT_INSTRUCTIONS_FILE: &str = "AGENTS.md";
/// The personal instructions file under the configured directory.
pub const PERSONAL_INSTRUCTIONS_FILE: &str = "instructions.md";

/// Where an instruction source lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstructionOrigin {
    /// The personal file under the explicitly configured user directory.
    Personal,
    /// The workspace-root file.
    Project,
}

/// Per-source and aggregate caps: resource limits, not inclusion promises.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstructionLimits {
    /// Cap on the admitted personal body.
    pub personal_max_bytes: usize,
    /// Cap on the admitted project body.
    pub project_max_bytes: usize,
    /// Cap on the two admitted bodies combined.
    pub total_max_bytes: usize,
}

/// One admitted source: the decoded body plus the whole file's identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstructionSource {
    pub origin: InstructionOrigin,
    pub file_name: &'static str,
    /// Valid UTF-8; possibly a truncated prefix of the file.
    pub body: String,
    /// The whole file's size in bytes, truncated or not.
    pub size: u64,
    /// Lowercase hex SHA-256 of the whole file, truncated or not.
    pub sha256: String,
    /// Whether the body is a prefix of the file.
    pub truncated: bool,
}

/// Why an enabled source contributed nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    SymlinkRefused,
    NotRegularFile,
    TooLarge,
    NotUtf8,
    Unreadable,
}

/// An enabled source that contributed nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkippedSource {
    pub origin: InstructionOrigin,
    pub reason: SkipReason,
}

/// The result of one bounded load: personal first, then project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedInstructions {
    pub sources: Vec<InstructionSource>,
    pub skipped: Vec<SkippedSource>,
}

/// Loads at most two instruction files. A `None` root or dir disables that
/// source and it is never touched; a missing file or missing source
/// directory is silent — neither a source nor a skip. Personal is read
/// first with `personal_max_bytes`; the project cap is the smaller of
/// `project_max_bytes` and `total_max_bytes` minus the admitted personal
/// body (saturating), so the aggregate never exceeds `total_max_bytes`.
/// Never panics, never errors: every refusal is data.
pub fn load_instructions(
    project_root: Option<&Path>,
    personal_dir: Option<&Path>,
    limits: &InstructionLimits,
) -> LoadedInstructions {
    let mut sources = Vec::new();
    let mut skipped = Vec::new();
    let personal_len = personal_dir.map_or(0, |dir| {
        read_one(
            dir,
            InstructionOrigin::Personal,
            PERSONAL_INSTRUCTIONS_FILE,
            limits.personal_max_bytes,
            &mut sources,
            &mut skipped,
        )
    });
    if let Some(root) = project_root {
        let cap = limits
            .total_max_bytes
            .saturating_sub(personal_len)
            .min(limits.project_max_bytes);
        read_one(
            root,
            InstructionOrigin::Project,
            PROJECT_INSTRUCTIONS_FILE,
            cap,
            &mut sources,
            &mut skipped,
        );
    }
    LoadedInstructions { sources, skipped }
}
