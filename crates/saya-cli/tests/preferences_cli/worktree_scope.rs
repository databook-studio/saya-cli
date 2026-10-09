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
