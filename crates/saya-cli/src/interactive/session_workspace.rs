//! Where an interactive session's workspace root binds, per
//! SESSION-WORKSPACE.md: the git worktree top containing the launch cwd,
//! canonicalised once, the launch cwd itself when it is a plain folder, or
//! an explicitly stated `--workspace <dir>`. A dangerous root (the
//! filesystem root, home, the temporary directory, or a mount root) with no
//! explicit statement binds nothing — write-shaped file tools stay hidden,
//! and the workspace reads deny with their typed error.
//!
//! One composition check belongs to the binding, not to any tool: a session
//! root must have no containment relation, in either direction, to the runs
//! root or the sessions root. The overrides (`SAYA_RUNS_DIR`,
//! `SAYA_SESSION_DIR`) can put either inside a project; without the check a
//! session's children — bounded to the project — could write active runs'
//! journals and their own session's state, and a state dir inside the root
//! puts those files within reach of the file tools. The
//! check is `place()`-shaped: refuse, naming the conflict, never silently
//! narrow.

use std::path::{Path, PathBuf};
use std::sync::Arc;

/// A bound session workspace: the canonical root and the opened seam.
pub(crate) struct SessionWorkspace {
    /// The canonical root — resolved once, pinned into the session record.
    pub(crate) root: PathBuf,
    /// The containment seam every file tool and download rides.
    pub(crate) workspace: Arc<saya_harness::workspace::Workspace>,
}

/// Resolves the workspace root: an explicit `--workspace <dir>` first (it
/// must exist; it is canonicalised and pinned like any root), else the git
/// worktree top above the launch cwd — the directory containing `.git`, a
/// directory or a worktree-link file — else the launch cwd itself, bound in
/// place. A dangerous root (the filesystem root, home, the temporary
/// directory, or a mount root) binds nothing.
pub(crate) fn resolve_root(explicit: Option<&Path>, cwd: &Path) -> Result<Option<PathBuf>, String> {
    resolve_root_with(
        explicit,
        cwd,
        home_directory().as_deref(),
        temp_directory().as_deref(),
    )
}

/// The resolution with its dangerous-root inputs stated, so the refusal is
/// testable without mutating the process environment.
fn resolve_root_with(
    explicit: Option<&Path>,
    cwd: &Path,
    home: Option<&Path>,
    temp: Option<&Path>,
) -> Result<Option<PathBuf>, String> {
    if let Some(dir) = explicit {
        let canonical = std::fs::canonicalize(dir).map_err(|error| {
            format!(
                "--workspace {} could not be resolved: {error}; name an existing directory",
                dir.display()
            )
        })?;
        if !canonical.is_dir() {
            return Err(format!(
                "--workspace {} is not a directory; name an existing directory",
                dir.display()
            ));
        }
        return Ok(Some(canonical));
    }
    let root = worktree_root(cwd).unwrap_or_else(|| cwd.to_path_buf());
    // Canonicalised once, here — the same rule an explicit bind follows —
    // so the pinned root is the resolved path, not the shell's spelling.
    let canonical = std::fs::canonicalize(root)
        .map_err(|error| format!("the workspace root could not be resolved: {error}"))?;
    if dangerous_root_reason(&canonical, home, temp).is_some() {
        return Ok(None);
    }
    Ok(Some(canonical))
}

/// Why a root must never be bound in place: capturing it would copy a
/// machine or a credential store into the session's reach.
fn dangerous_root_reason(
    path: &Path,
    home: Option<&Path>,
    temp: Option<&Path>,
) -> Option<&'static str> {
    if path == Path::new("/") {
        Some("the filesystem root")
    } else if home == Some(path) {
        Some("the home directory")
    } else if temp == Some(path) {
        Some("the temporary directory")
    } else if is_mount_root(path) {
        Some("a mount root")
    } else {
        None
    }
}

/// A directory whose device differs from its parent's is the root of a
/// mounted filesystem; binding it would cross an arbitrary boundary.
#[cfg(unix)]
fn is_mount_root(path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt as _;
    let (Some(parent), Ok(metadata)) = (path.parent(), std::fs::metadata(path)) else {
        return false;
    };
    std::fs::metadata(parent).is_ok_and(|parent| parent.dev() != metadata.dev())
}

#[cfg(not(unix))]
fn is_mount_root(_path: &Path) -> bool {
    false
}

/// Binds the resolved root: the composition check against the state roots,
/// then the seam. `None` means nothing bound — the session's honest
/// read-only-by-default shape.
pub(crate) fn bind(
    explicit: Option<&Path>,
    cwd: &Path,
) -> Result<Option<SessionWorkspace>, String> {
    bind_with_state_roots(explicit, cwd, &state_roots())
}

/// The binding with its state roots stated, so the composition refusal is
/// testable without mutating the process environment.
fn bind_with_state_roots(
    explicit: Option<&Path>,
    cwd: &Path,
    state_roots: &[PathBuf],
) -> Result<Option<SessionWorkspace>, String> {
    let Some(root) = resolve_root(explicit, cwd)? else {
        return Ok(None);
    };
    for state_root in state_roots {
        check_state_overlap(&root, state_root)?;
    }
    let workspace = saya_harness::workspace::Workspace::open(&root).map_err(|error| {
        format!(
            "the workspace root {} could not be opened: {error}",
            root.display()
        )
    })?;
    Ok(Some(SessionWorkspace {
        root,
        workspace: Arc::new(workspace),
    }))
}

/// The fresh-unbound fact, said on the composition notice seam at startup:
/// a dangerous root with no `--workspace` binds nothing — so the
/// write-shaped file tools stay hidden and workspace reads refuse — and the
/// session names the absence and the remedy rather than staying silent.
pub(crate) const NO_WORKSPACE_NOTICE: &str = "No workspace is bound (the launch directory is a filesystem \
    root, home, the temporary directory, or a mount root): file tools are unavailable; launch inside a \
    project folder or pass `--workspace <dir>`.";

/// The plain-folder binding fact: a session launched outside any worktree is
/// bound to that folder in place, with no Git history behind it.
pub(crate) fn in_place_notice(root: &Path) -> String {
    format!(
        "No Git work tree found; the session is bound to {} in place.",
        root.display()
    )
}

/// The Git-root in-place coding fact: the default root edits the worktree
/// itself, so a fresh session names it and the rewind remedy. A plain folder
/// returns `None` — its [`in_place_notice`] already carries the fact.
pub(crate) fn coding_in_place_notice(root: &Path) -> Option<String> {
    worktree_root(root).is_some().then(|| {
        format!(
            "Jobs edit {} in place: edits land in this folder; `/job rewind` undoes tool edits.",
            root.display()
        )
    })
}

/// Binds the session universe's workspace from its three statements: an
/// explicit `--workspace` first; on a resume, the recorded pin; on a fresh
/// session, the git worktree top, else the launch cwd bound in place. A
/// recorded root that no longer exists binds nothing and says so — fail
/// closed, never re-derive-and-hope. The returned notice is the composition
/// fact the session must say at startup.
pub(crate) fn bind_from_pins(
    explicit: Option<&Path>,
    pinned_root: Option<&str>,
    walk_when_unpinned: bool,
    cwd: &Path,
) -> Result<(Option<SessionWorkspace>, Option<String>), String> {
    if let Some(dir) = explicit {
        let bound = bind(Some(dir), cwd)?.expect("an explicit bind returns the root");
        return Ok((Some(bound), None));
    }
    let Some(pin) = pinned_root else {
        let plain = worktree_root(cwd).is_none();
        let bound = if walk_when_unpinned {
            bind(None, cwd)?
        } else {
            // A resumed session whose record predates the workspace: it
            // resumes unbound, never re-derived from wherever the shell
            // happens to be.
            None
        };
        let notice = match (&bound, walk_when_unpinned) {
            // A fresh session that bound a plain folder says so at startup;
            // a fresh session that bound nothing names the refusal; a
            // pre-workspace resume keeps its old silence.
            (Some(bound), true) if plain => Some(in_place_notice(&bound.root)),
            (None, true) => Some(NO_WORKSPACE_NOTICE.to_owned()),
            _ => None,
        };
        return Ok((bound, notice));
    };
    let recorded = PathBuf::from(pin);
    if !recorded.exists() {
        return Ok((
            None,
            Some(format!(
                "the recorded workspace root {pin} no longer exists: no workspace is bound, \
                 so file reads and writes are unavailable this session"
            )),
        ));
    }
    Ok((bind(Some(&recorded), cwd)?, None))
}

/// Walks up from `start` looking for a `.git` directory or worktree file;
/// the directory containing it is the worktree top. No new dependency, and
/// `git worktree`/submodule layouts resolve to the tree actually entered.
fn worktree_root(start: &Path) -> Option<PathBuf> {
    let mut current = Some(start.to_path_buf());
    while let Some(dir) = current {
        if dir.join(".git").exists() {
            return Some(dir);
        }
        current = dir.parent().map(Path::to_path_buf);
    }
    None
}

/// The state roots a session root must stay disjoint from, read from the
/// environment at bind time.
fn state_roots() -> Vec<PathBuf> {
    [("SAYA_RUNS_DIR", "runs"), ("SAYA_SESSION_DIR", "sessions")]
        .into_iter()
        .filter_map(|(env, subdir)| state_root_for_check(env, subdir))
        .collect()
}

/// The user's home directory, canonical when it exists.
fn home_directory() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .and_then(|home| std::fs::canonicalize(home).ok())
}

/// The system temporary directory, canonical when it exists.
fn temp_directory() -> Option<PathBuf> {
    std::fs::canonicalize(std::env::temp_dir()).ok()
}

/// One state root to check, as the resolution chain states it: an override
/// is deliberate wherever it points and is made absolute against the launch
/// cwd; a data home (XDG, then APPDATA, then HOME) is a stated location.
/// When the environment states none of these — a machine with no data home
/// at all — there is no root the operator chose, and the resolution's
/// relative fallback is a launch-cwd accident, not a bindable location: it
/// is skipped rather than turned into a refusal that no setting could
/// answer.
fn state_root_for_check(override_env: &str, subdir: &str) -> Option<PathBuf> {
    if let Some(value) = std::env::var_os(override_env) {
        return Some(canonical_or_absolute(Path::new(&value)));
    }
    for (env, leaf) in [
        ("XDG_DATA_HOME", subdir),
        ("APPDATA", subdir),
        ("HOME", subdir),
    ] {
        if let Some(value) = std::env::var_os(env) {
            return Some(canonical_or_absolute(
                &Path::new(&value).join("saya").join(leaf),
            ));
        }
    }
    None
}

/// The composition check itself, testable with explicit roots: a workspace
/// root with any containment relation — in either direction — to one state
/// root refuses, naming both.
fn check_state_overlap(root: &Path, state_root: &Path) -> Result<(), String> {
    {
        if root.starts_with(state_root) || state_root.starts_with(root) {
            return Err(format!(
                "the workspace root {} cannot be bound: it overlaps the saya state root {} — \
                 a session's children are bounded to the workspace root, so a state root \
                 inside it would put the runs' journals or this session's scratch, lock, and \
                 transcript within their reach, and a workspace inside a state root would \
                 put the workspace under the file tools' reach; set SAYA_RUNS_DIR / \
                 SAYA_SESSION_DIR outside the workspace tree",
                root.display(),
                state_root.display()
            ));
        }
    }
    Ok(())
}

/// The canonical form of a state root for containment comparison: the
/// canonical path when it exists, otherwise its absolute lexical form.
fn canonical_or_absolute(path: &Path) -> PathBuf {
    if let Ok(canonical) = std::fs::canonicalize(path) {
        return canonical;
    }
    if path.is_absolute() {
        return path.to_path_buf();
    }
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(path)
}

#[cfg(test)]
#[path = "session_workspace_tests.rs"]
mod tests;
