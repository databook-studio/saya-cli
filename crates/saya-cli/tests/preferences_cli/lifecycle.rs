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
