//! The workspace binding's launch-time tests.

use std::fs;
use std::path::{Path, PathBuf};

use super::{
    SessionWorkspace, bind, bind_from_pins, bind_with_state_roots, check_state_overlap,
    resolve_root, resolve_root_with,
};

fn temp_dir(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("saya-session-ws-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

/// An explicit `--workspace <dir>` binds that directory, canonicalised —
/// the user's statement wins, whatever the launch cwd is.
#[test]
fn an_explicit_workspace_binds_canonically() {
    let dir = temp_dir("explicit");
    let SessionWorkspace { root, workspace } = bind(Some(Path::new(&dir)), Path::new("/"))
        .expect("explicit bind")
        .expect("the root binds");
    assert_eq!(root, std::fs::canonicalize(&dir).unwrap());
    assert_eq!(workspace.root(), std::fs::canonicalize(&dir).unwrap());
    let _ = fs::remove_dir_all(&dir);
}

/// A `--workspace` that does not exist refuses: the pin, not a guess, is
/// what carries the invariant.
#[test]
fn an_explicit_workspace_must_exist() {
    let parent = temp_dir("missing");
    let missing = parent.join("no-such");
    let error = bind(Some(Path::new(&missing)), Path::new("/"))
        .map(|bound| bound.is_some())
        .expect_err("a nonexistent explicit root must refuse");
    assert!(
        error.contains("could not be resolved"),
        "the refusal must name the resolution failure: {error}"
    );
    let _ = fs::remove_dir_all(&parent);
}

/// Outside a git worktree with no explicit statement, the launch cwd binds
/// as a plain folder — decision 1: a plain folder is the session workspace.
#[test]
fn plain_cwd_binds() {
    let dir = temp_dir("no-worktree");
    let nested = dir.join("deep").join("deeper");
    std::fs::create_dir_all(&nested).unwrap();
    let bound = bind(None, &nested)
        .expect("the resolution runs")
        .expect("a plain folder binds in place");
    assert_eq!(bound.root, std::fs::canonicalize(&nested).unwrap());
    assert_eq!(bound.workspace.root(), bound.root);
    let _ = fs::remove_dir_all(&dir);
}

/// The user's home directory is dangerous: capturing it would copy a
/// credential store into the baseline. It binds nothing, and the notice
/// names the refusal rather than pretending the folder is absent.
#[test]
fn home_cwd_refused() {
    let dir = temp_dir("home");
    let control = temp_dir("home-control");
    let home = std::fs::canonicalize(&dir).unwrap();
    // A plain folder still binds...
    assert!(
        resolve_root_with(None, &control, Some(&home), None)
            .expect("the resolution runs")
            .is_some(),
        "a plain folder beside home binds in place"
    );
    // ...but home itself binds nothing.
    let resolved = resolve_root_with(None, &dir, Some(&home), None).expect("the resolution runs");
    assert!(
        resolved.is_none(),
        "the home directory binds nothing in place"
    );
    let _ = fs::remove_dir_all(&control);
    let _ = fs::remove_dir_all(&dir);
}

/// A plain folder that overlaps a saya state root refuses the binding: the
/// state roots stay outside the file tools' reach.
#[test]
fn state_overlap_refused() {
    let state = temp_dir("overlap-plain");
    let project = state.join("project");
    std::fs::create_dir_all(&project).unwrap();
    let state = std::fs::canonicalize(&state).unwrap();
    let error = bind_with_state_roots(None, &project, std::slice::from_ref(&state))
        .map(|bound| bound.is_some())
        .expect_err("a plain folder inside a state root must refuse");
    assert!(
        error.contains(state.display().to_string().as_str()),
        "the refusal must name the state root: {error}"
    );
    let _ = fs::remove_dir_all(&state);
}

/// A resume keeps its recorded pin: the shell's plain cwd is never
/// re-derived over it, and no in-place notice is said for the pin.
#[test]
fn resume_pin_unchanged() {
    let project = temp_dir("resume-pin");
    std::fs::create_dir_all(project.join(".git")).unwrap();
    let plain = temp_dir("resume-cwd");
    // A fresh session now binds the plain folder in place...
    assert_eq!(
        resolve_root(None, &plain).expect("resolves"),
        Some(std::fs::canonicalize(&plain).unwrap()),
        "the fresh session binds its plain cwd"
    );
    // ...but a resume keeps the recorded pin whatever the shell's cwd is.
    let pin = std::fs::canonicalize(&project).unwrap();
    let (bound, notice) = bind_from_pins(None, Some(pin.to_str().unwrap()), true, &plain)
        .expect("the pinned resume runs");
    assert_eq!(bound.expect("the pin binds").root, pin);
    assert_eq!(notice, None, "a pinned resume says no in-place notice");
    let _ = fs::remove_dir_all(&project);
    let _ = fs::remove_dir_all(&plain);
}

/// Inside a git worktree, the worktree top binds — from a subdirectory too:
/// the launch binding pin (the root is the worktree top, not the cwd).
#[test]
fn the_worktree_top_binds_from_a_subdirectory() {
    let project = temp_dir("worktree");
    std::fs::create_dir_all(project.join(".git")).unwrap();
    let nested = project.join("src").join("sub");
    std::fs::create_dir_all(&nested).unwrap();
    let bound = bind(None, &nested).expect("the resolution runs");
    let bound = bound.expect("the worktree top binds");
    assert_eq!(bound.root, std::fs::canonicalize(&project).unwrap());
    let _ = fs::remove_dir_all(&project);
}

/// A `.git` *file* (a worktree's pointer file) binds its containing
/// directory: `git worktree` layouts resolve to the tree entered.
#[test]
fn a_worktree_pointer_file_binds() {
    let main = temp_dir("main-repo");
    let linked = temp_dir("linked-worktree");
    std::fs::create_dir_all(main.join(".git")).unwrap();
    std::fs::write(linked.join(".git"), "gitdir: elsewhere\n").unwrap();
    let bound = bind(None, &linked).expect("the resolution runs");
    let bound = bound.expect("the worktree binds");
    assert_eq!(bound.root, std::fs::canonicalize(&linked).unwrap());
    let _ = fs::remove_dir_all(&main);
    let _ = fs::remove_dir_all(&linked);
}

/// The composition check: a workspace root that contains a state root —
/// `SAYA_RUNS_DIR` or `SAYA_SESSION_DIR` redirected inside a project —
/// refuses the binding, naming both roots, because the children are bounded
/// to the root and the state roots would be within their reach.
#[test]
fn a_state_root_inside_the_workspace_refuses_the_binding() {
    let project = temp_dir("overlap");
    std::fs::create_dir_all(project.join(".git")).unwrap();
    let runs = project.join("runs");
    let sessions = project.join(".saya-sessions");
    std::fs::create_dir_all(&runs).unwrap();
    std::fs::create_dir_all(&sessions).unwrap();
    let canonical = std::fs::canonicalize(&project).unwrap();
    for state_root in [&runs, &sessions] {
        let state_canonical = std::fs::canonicalize(state_root).unwrap();
        let error = check_state_overlap(&canonical, &state_canonical)
            .expect_err("the state root inside the project must refuse");
        assert!(
            error.contains(canonical.display().to_string().as_str()),
            "the refusal must name the workspace root: {error}"
        );
        assert!(
            error.contains(state_canonical.display().to_string().as_str()),
            "the refusal must name the state root: {error}"
        );
    }
    let _ = fs::remove_dir_all(&project);
}

/// The other direction: a workspace root inside a state root refuses too —
/// there the state root would be inside the file tools' reach.
#[test]
fn the_workspace_inside_a_state_root_refuses_the_binding() {
    let state = temp_dir("state-root");
    let project = state.join("proj");
    std::fs::create_dir_all(&project).unwrap();
    let error = check_state_overlap(&project, &state)
        .expect_err("a workspace inside a state root must refuse");
    assert!(
        error.contains(project.display().to_string().as_str())
            && error.contains(state.display().to_string().as_str()),
        "the refusal must name both roots: {error}"
    );
    let _ = fs::remove_dir_all(&state);
}

/// Disjoint roots bind: the default layout (state roots beside each other
/// in the data home, projects elsewhere) is disjoint by construction.
#[test]
fn disjoint_roots_bind() {
    let project = temp_dir("disjoint");
    let state = temp_dir("disjoint-state");
    check_state_overlap(&project, &state).expect("disjoint binds");
    let _ = fs::remove_dir_all(&project);
    let _ = fs::remove_dir_all(&state);
}

/// `resolve_root` mirrors `bind`'s resolution: explicit wins, worktree
/// second, nothing otherwise. Pinned separately so the pin logic in the
/// session loop cannot drift from the binding itself.
#[test]
fn resolve_root_mirrors_the_binding() {
    let project = temp_dir("resolve");
    std::fs::create_dir_all(project.join(".git")).unwrap();
    assert_eq!(
        resolve_root(Some(Path::new(&project)), Path::new("/"))
            .expect("explicit resolves")
            .expect("explicit binds"),
        std::fs::canonicalize(&project).unwrap()
    );
    assert!(resolve_root(None, &project).expect("resolves").is_some());
    let plain = temp_dir("resolve-plain");
    assert_eq!(
        resolve_root(None, &plain).expect("resolves"),
        Some(std::fs::canonicalize(&plain).unwrap()),
        "a plain folder resolves to itself"
    );
    let _ = fs::remove_dir_all(&project);
    let _ = fs::remove_dir_all(&plain);
}
