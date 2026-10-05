//! `saya preferences` through the real binary (M04a).
//!
//! Every preference is explicit, per-worktree guidance the user confirmed by
//! running a command: a fresh worktree lists nothing, adds and edits and
//! forgets behave and report, secret-shaped text is refused and stored
//! nowhere, another worktree sees none of this worktree's preferences, a
//! run from a subdirectory of the same worktree sees the same ones (the
//! identity is the canonical root, not the launch cwd), and a run outside
//! any git worktree is refused with the worktree remedy. Every refusal
//! exits 2, the documented usage class. No path reads stdin.

use std::{
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

fn unique_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "saya-cli-preferences-{label}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A fake worktree for the binary: a directory holding an empty `.git`
/// directory — the walk-up in the workspace resolver stops exactly there.
fn fake_worktree(root: &Path, name: &str) -> PathBuf {
    let dir = root.join(name);
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    dir
}

fn saya(root: &Path, cwd: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_saya"));
    command
        .args(args)
        .current_dir(cwd)
        .env("SAYA_STATE_DB", root.join("state.sqlite3"))
        .env("SAYA_CONFIG_HOME", root.join("user-config"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

fn run(root: &Path, cwd: &Path, args: &[&str]) -> Output {
    saya(root, cwd, args).output().unwrap()
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The first `pp-…` token in a listing — 35 chars, `pp-` plus 32 hex.
fn first_preference_id(text: &str) -> Option<String> {
    text.split_whitespace()
        .find(|token| token.len() == 35 && token.starts_with("pp-"))
        .map(str::to_owned)
}

fn cleanup(root: &Path) {
    let _ = std::fs::remove_dir_all(root);
}

/// A fresh worktree lists nothing and hints at the add form.
#[test]
fn list_in_a_fresh_worktree_prints_the_empty_message() {
    let root = unique_dir("empty");
    let wt = fake_worktree(&root, "wt");
    let output = run(&root, &wt, &["preferences", "list"]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "stdout: {}\nstderr: {}",
        stdout_of(&output),
        stderr_of(&output)
    );
    let out = stdout_of(&output);
    assert!(out.contains("No project preferences for "), "{out}");
    assert!(
        out.contains(concat!("saya preferences add ", "\"", "<text>", "\"")),
        "the empty listing names the add form: {out}"
    );
    cleanup(&root);
}

/// `add` exits 0 and `list` shows the text with a `pp-` id.
#[test]
fn add_then_list_shows_the_preference_with_a_pp_id() {
    let root = unique_dir("add-list");
    let wt = fake_worktree(&root, "wt");
    let added = run(&root, &wt, &["preferences", "add", "prefer tabs"]);
    assert_eq!(
        added.status.code(),
        Some(0),
        "stdout: {}\nstderr: {}",
        stdout_of(&added),
        stderr_of(&added)
    );
    let listing = run(&root, &wt, &["preferences", "list"]);
    assert_eq!(listing.status.code(), Some(0));
    let out = stdout_of(&listing);
    assert!(out.contains("prefer tabs"), "{out}");
    assert!(
        first_preference_id(&out).is_some(),
        "the listing carries a pp- id: {out}"
    );
    assert!(
        out.contains("They never grant permissions"),
        "the retention sentence rides the listing: {out}"
    );
    cleanup(&root);
}

/// `edit <id>` replaces the text; the old text disappears from the listing.
#[test]
fn edit_changes_the_text() {
    let root = unique_dir("edit");
    let wt = fake_worktree(&root, "wt");
    run(&root, &wt, &["preferences", "add", "prefer tabs"]);
    let listing = run(&root, &wt, &["preferences", "list"]);
    let id = first_preference_id(&stdout_of(&listing)).expect("an id after add");
    let edited = run(&root, &wt, &["preferences", "edit", &id, "prefer spaces"]);
    assert_eq!(
        edited.status.code(),
        Some(0),
        "stdout: {}\nstderr: {}",
        stdout_of(&edited),
        stderr_of(&edited)
    );
    let out = stdout_of(&run(&root, &wt, &["preferences", "list"]));
    assert!(out.contains("prefer spaces"), "{out}");
    assert!(!out.contains("prefer tabs"), "the old text is gone: {out}");
    cleanup(&root);
}

/// `forget <id>` removes the preference; a second forget of the same id is a
/// refusal (exit 2) that states nothing was removed — decided here: a
/// missing id is a refusal, consistent with `sessions delete` and
/// `investigation delete`, and it changes nothing.
#[test]
fn forget_removes_it_and_a_second_forget_refuses() {
    let root = unique_dir("forget");
    let wt = fake_worktree(&root, "wt");
    run(&root, &wt, &["preferences", "add", "prefer tabs"]);
    let id = first_preference_id(&stdout_of(&run(&root, &wt, &["preferences", "list"])))
        .expect("an id after add");

    let forgotten = run(&root, &wt, &["preferences", "forget", &id]);
    assert_eq!(
        forgotten.status.code(),
        Some(0),
        "stdout: {}\nstderr: {}",
        stdout_of(&forgotten),
        stderr_of(&forgotten)
    );
    let out = stdout_of(&run(&root, &wt, &["preferences", "list"]));
    assert!(out.contains("No project preferences for "), "{out}");

    let again = run(&root, &wt, &["preferences", "forget", &id]);
    assert_eq!(
        again.status.code(),
        Some(2),
        "a missing id is a refusal: stdout {}\nstderr {}",
        stdout_of(&again),
        stderr_of(&again)
    );
    assert!(
        stderr_of(&again).contains("no preference"),
        "the refusal says nothing was removed: {}",
        stderr_of(&again)
    );
    cleanup(&root);
}

/// Secret-shaped text is refused (exit 2) and nothing is stored.
#[test]
fn secret_shaped_add_is_refused_and_nothing_is_stored() {
    let root = unique_dir("secret");
    let wt = fake_worktree(&root, "wt");
    let refused = run(&root, &wt, &["preferences", "add", "api_key=abc123"]);
    assert_eq!(
        refused.status.code(),
        Some(2),
        "stdout: {}\nstderr: {}",
        stdout_of(&refused),
        stderr_of(&refused)
    );
    assert!(
        stderr_of(&refused).contains("not accepted"),
        "{}",
        stderr_of(&refused)
    );
    let out = stdout_of(&run(&root, &wt, &["preferences", "list"]));
    assert!(out.contains("No project preferences for "), "{out}");
    cleanup(&root);
}

/// A second worktree (its own `.git`) sees nothing of the first worktree's
/// preferences — not on list, not on edit, not on forget.
#[test]
fn another_worktree_sees_nothing() {
    let root = unique_dir("isolation");
    let wa = fake_worktree(&root, "wt-a");
    let wb = fake_worktree(&root, "wt-b");
    run(&root, &wa, &["preferences", "add", "worktree a only"]);
    let id = first_preference_id(&stdout_of(&run(&root, &wa, &["preferences", "list"])))
        .expect("an id after add");

    let out_b = stdout_of(&run(&root, &wb, &["preferences", "list"]));
    assert!(
        out_b.contains("No project preferences for "),
        "B lists nothing: {out_b}"
    );
    let edited = run(&root, &wb, &["preferences", "edit", &id, "hijack"]);
    assert_eq!(edited.status.code(), Some(2), "{}", stderr_of(&edited));
    let forgotten = run(&root, &wb, &["preferences", "forget", &id]);
    assert_eq!(
        forgotten.status.code(),
        Some(2),
        "{}",
        stderr_of(&forgotten)
    );
    let out_a = stdout_of(&run(&root, &wa, &["preferences", "list"]));
    assert!(out_a.contains("worktree a only"), "A kept its row: {out_a}");
    cleanup(&root);
}

/// Running from a SUBDIRECTORY of the worktree sees the same preferences:
/// the identity is the canonical worktree root, not the launch directory.
/// This is the mutation guard for the identity derivation.
#[test]
fn a_subdirectory_of_the_worktree_sees_the_same_preferences() {
    let root = unique_dir("subdir");
    let wt = fake_worktree(&root, "wt");
    std::fs::create_dir_all(wt.join("src").join("deep")).unwrap();
    run(&root, &wt, &["preferences", "add", "prefer tabs"]);
    let out = stdout_of(&run(
        &root,
        &wt.join("src").join("deep"),
        &["preferences", "list"],
    ));
    assert_eq!(
        run(
            &root,
            &wt.join("src").join("deep"),
            &["preferences", "list"]
        )
        .status
        .code(),
        Some(0)
    );
    assert!(
        out.contains("prefer tabs"),
        "same worktree from a subdirectory: {out}"
    );
    cleanup(&root);
}

/// Outside any git worktree the command refuses with the worktree remedy.
/// The temp dir is verified ancestor-by-ancestor first; if any ancestor
/// carries a `.git` (this checkout was placed under one), the assertion is
/// skipped with an explicit message rather than passing by luck.
#[test]
fn running_outside_any_worktree_is_refused_with_the_remedy() {
    let root = unique_dir("no-worktree");
    let plain = root.join("plain");
    std::fs::create_dir_all(&plain).unwrap();
    let mut ancestor = plain.as_path();
    let mut has_git_ancestor = false;
    while let Some(parent) = ancestor.parent() {
        if ancestor.join(".git").exists() {
            has_git_ancestor = true;
            break;
        }
        ancestor = parent;
    }
    if has_git_ancestor || plain.join(".git").exists() {
        eprintln!(
            "SKIP running_outside_any_worktree_is_refused_with_the_remedy: \
             an ancestor of {} carries a .git, so no .git-free cwd exists here",
            plain.display()
        );
        cleanup(&root);
        return;
    }
    let refused = run(&root, &plain, &["preferences", "list"]);
    assert_eq!(
        refused.status.code(),
        Some(2),
        "stdout: {}\nstderr: {}",
        stdout_of(&refused),
        stderr_of(&refused)
    );
    assert!(
        stderr_of(&refused).contains("no project worktree here"),
        "{}",
        stderr_of(&refused)
    );
    cleanup(&root);
}

/// `saya preferences --help` states the contract: explicit, per-worktree
/// guidance, never permissions.
#[test]
fn help_mentions_never_permissions() {
    let root = unique_dir("help");
    let help = run(&root, &root, &["preferences", "--help"]);
    assert_eq!(help.status.code(), Some(0), "{}", stderr_of(&help));
    let out = stdout_of(&help);
    assert!(out.contains("never permissions"), "{out}");
    for subcommand in ["list", "add", "edit", "forget", "forget-all"] {
        assert!(out.contains(subcommand), "help names {subcommand}: {out}");
    }
    cleanup(&root);
}

/// Independence smoke: `saya --help` still presents the whole surface
/// (typed database knowledge under `contracts` and preferences beside it),
/// with no database profile needed anywhere in this file.
#[test]
fn saya_help_smoke_shows_both_surfaces() {
    let root = unique_dir("smoke");
    let help = run(&root, &root, &["--help"]);
    assert_eq!(help.status.code(), Some(0), "{}", stderr_of(&help));
    let out = stdout_of(&help);
    assert!(out.contains("preferences"), "{out}");
    assert!(out.contains("contracts"), "{out}");
    cleanup(&root);
}

/// The command never reads stdin: with an open, empty pipe held open, a
/// `preferences list` completes on its own instead of blocking forever.
#[test]
fn the_command_never_reads_stdin() {
    let root = unique_dir("no-stdin");
    let wt = fake_worktree(&root, "wt");
    let mut child = saya(&root, &wt, &["preferences", "list"])
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    // Held open and never written: any stdin read would block forever.
    let _held_open = child.stdin.take();
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the command blocked waiting on stdin"
        );
        std::thread::sleep(Duration::from_millis(100));
    };
    assert_eq!(status.code(), Some(0), "list completed without stdin");
    let mut stdout = String::new();
    use std::io::Read;
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut stdout)
        .unwrap();
    assert!(stdout.contains("No project preferences for "), "{stdout}");
    cleanup(&root);
}
