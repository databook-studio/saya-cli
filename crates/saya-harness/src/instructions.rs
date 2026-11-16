//! The bounded instruction loader: a personal `instructions.md` inside an
//! explicitly configured directory, then project files — the workspace-root
//! `AGENTS.md` (or `CLAUDE.md` when `AGENTS.md` is absent) and the same pair
//! in the directories between the root and the current working directory —
//! read through the contained, no-follow [`Workspace::read`]. No discovery
//! walk, no import handling, no command execution: bodies return literally,
//! as context data that grants nothing.

use std::path::{Component, Path};

use self::read::{ReadOutcome, read_one};
use crate::workspace::Workspace;

mod read;

/// The project instructions file. `AGENTS.md` wins when both names exist.
pub const PROJECT_INSTRUCTIONS_FILE: &str = "AGENTS.md";
/// The fallback project instructions file, read only when `AGENTS.md` is
/// absent from the same directory.
pub const CLAUDE_INSTRUCTIONS_FILE: &str = "CLAUDE.md";
/// The personal instructions file under the configured directory.
pub const PERSONAL_INSTRUCTIONS_FILE: &str = "instructions.md";
/// Cap on nested project files, so a deep working directory cannot make the
/// loader read an unbounded number of files.
pub const MAX_NESTED_INSTRUCTION_FILES: usize = 64;

/// Where an instruction source lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstructionOrigin {
    /// The personal file under the explicitly configured user directory.
    Personal,
    /// A project file: the workspace root or a directory on the path to the
    /// current working directory.
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
    /// The name that was read: `AGENTS.md`, `CLAUDE.md`, or `instructions.md`.
    pub file_name: &'static str,
    /// The workspace-relative path that was read, e.g. `sub/AGENTS.md`.
    pub path: String,
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
    /// The name that was attempted.
    pub file_name: &'static str,
    pub reason: SkipReason,
}

/// The result of one bounded load: personal first, then project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedInstructions {
    pub sources: Vec<InstructionSource>,
    pub skipped: Vec<SkippedSource>,
}

/// Loads the personal file and the project files under one aggregate cap.
///
/// A `None` root or dir disables that source and it is never touched; a
/// missing file or missing source directory is silent — neither a source nor
/// a skip. Personal is read first with `personal_max_bytes`. The project
/// files then load in order — the root first, then the directories between
/// the root and `working_dir` (inclusive), top-down — each capped by
/// `project_max_bytes` and by what remains of that same project allowance,
/// so every project file together never exceeds the single-file bound; the
/// aggregate never exceeds `total_max_bytes` either, and every cut is stated
/// by [`InstructionSource::truncated`]. A directory contributes `AGENTS.md`,
/// or `CLAUDE.md` only when `AGENTS.md` is absent; when both exist,
/// `AGENTS.md` wins and `CLAUDE.md` is ignored without an error. Nested
/// files are bounded by [`MAX_NESTED_INSTRUCTION_FILES`]. Every read is a
/// contained, no-follow read of the opened workspace, so a symlinked file or
/// directory component is refused rather than followed out of the workspace.
/// Never panics, never errors: every refusal is data.
pub fn load_instructions(
    project_root: Option<&Path>,
    working_dir: Option<&Path>,
    personal_dir: Option<&Path>,
    limits: &InstructionLimits,
) -> LoadedInstructions {
    let mut sources = Vec::new();
    let mut skipped = Vec::new();
    let personal_len = personal_dir.map_or(0, |dir| {
        read_personal(dir, limits.personal_max_bytes, &mut sources, &mut skipped)
    });
    let Some(root) = project_root else {
        return LoadedInstructions { sources, skipped };
    };
    let workspace = match Workspace::open(root) {
        Ok(workspace) => workspace,
        Err(error) if error.is_not_found() => return LoadedInstructions { sources, skipped },
        Err(_) => {
            // The configured root could not be opened as a directory; the
            // refusal is about the directory, not about the file inside it.
            skipped.push(SkippedSource {
                origin: InstructionOrigin::Project,
                file_name: PROJECT_INSTRUCTIONS_FILE,
                reason: SkipReason::Unreadable,
            });
            return LoadedInstructions { sources, skipped };
        }
    };
    // Project files share the single-file allowance: the existing bound is
    // the aggregate, so nested files cannot add bytes beyond it even when the
    // total leaves room.
    let mut remaining = limits
        .project_max_bytes
        .min(limits.total_max_bytes.saturating_sub(personal_len));
    // The root is read even at a zero remainder, so an aggregate cap that
    // the personal file consumed still states the root file's truncation.
    let root_cap = limits.project_max_bytes.min(remaining);
    let admitted = read_project(&workspace, "", root_cap, &mut sources, &mut skipped);
    remaining = remaining.saturating_sub(admitted);
    if remaining == 0 {
        return LoadedInstructions { sources, skipped };
    }
    for directory in nested_directories(workspace.root(), working_dir)
        .into_iter()
        .take(MAX_NESTED_INSTRUCTION_FILES)
    {
        let cap = limits.project_max_bytes.min(remaining);
        let admitted = read_project(&workspace, &directory, cap, &mut sources, &mut skipped);
        remaining = remaining.saturating_sub(admitted);
        if remaining == 0 {
            break;
        }
    }
    LoadedInstructions { sources, skipped }
}

/// Reads one directory's project file, trying the fallback name only when
/// `AGENTS.md` is absent. Returns the admitted byte length.
fn read_project(
    workspace: &Workspace,
    directory: &str,
    cap: usize,
    sources: &mut Vec<InstructionSource>,
    skipped: &mut Vec<SkippedSource>,
) -> usize {
    let agents = join(directory, PROJECT_INSTRUCTIONS_FILE);
    match read_one(
        workspace,
        &agents,
        InstructionOrigin::Project,
        PROJECT_INSTRUCTIONS_FILE,
        cap,
        sources,
        skipped,
    ) {
        ReadOutcome::Absent => {
            let claude = join(directory, CLAUDE_INSTRUCTIONS_FILE);
            match read_one(
                workspace,
                &claude,
                InstructionOrigin::Project,
                CLAUDE_INSTRUCTIONS_FILE,
                cap,
                sources,
                skipped,
            ) {
                ReadOutcome::Admitted(len) => len,
                ReadOutcome::Absent | ReadOutcome::Skipped => 0,
            }
        }
        ReadOutcome::Admitted(len) => len,
        ReadOutcome::Skipped => 0,
    }
}

fn read_personal(
    dir: &Path,
    max_bytes: usize,
    sources: &mut Vec<InstructionSource>,
    skipped: &mut Vec<SkippedSource>,
) -> usize {
    let workspace = match Workspace::open(dir) {
        Ok(workspace) => workspace,
        // A missing directory is absent, not broken: the normal case for a
        // user with no personal configuration, silent like a missing file.
        Err(error) if error.is_not_found() => return 0,
        Err(_) => {
            // The configured personal directory could not be opened; the
            // refusal is about the directory, not about `instructions.md`.
            skipped.push(SkippedSource {
                origin: InstructionOrigin::Personal,
                file_name: PERSONAL_INSTRUCTIONS_FILE,
                reason: SkipReason::Unreadable,
            });
            return 0;
        }
    };
    match read_one(
        &workspace,
        PERSONAL_INSTRUCTIONS_FILE,
        InstructionOrigin::Personal,
        PERSONAL_INSTRUCTIONS_FILE,
        max_bytes,
        sources,
        skipped,
    ) {
        ReadOutcome::Admitted(len) => len,
        ReadOutcome::Absent | ReadOutcome::Skipped => 0,
    }
}

/// The workspace-relative directories from the root to `working_dir`,
/// top-down and including `working_dir` itself. A working directory outside
/// the root, or one that cannot be resolved, yields none.
fn nested_directories(root: &Path, working_dir: Option<&Path>) -> Vec<String> {
    let Some(working_dir) = working_dir else {
        return Vec::new();
    };
    let Ok(working_dir) = working_dir.canonicalize() else {
        return Vec::new();
    };
    let Ok(relative) = working_dir.strip_prefix(root) else {
        return Vec::new();
    };
    let mut directories = Vec::new();
    let mut current = String::new();
    for component in relative.components() {
        let Component::Normal(part) = component else {
            return Vec::new();
        };
        let Some(part) = part.to_str() else {
            return Vec::new();
        };
        if !current.is_empty() {
            current.push('/');
        }
        current.push_str(part);
        directories.push(current.clone());
    }
    directories
}

fn join(directory: &str, name: &str) -> String {
    if directory.is_empty() {
        name.to_owned()
    } else {
        format!("{directory}/{name}")
    }
}
