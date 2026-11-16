/// `/preferences list` in a session with no bound root: the shared
/// NoWorktree refusal as an Error block, the exact words the headless CLI
/// prints — never an empty listing, never the launch cwd.
#[test]
fn preferences_dispatch_refuses_without_a_bound_root() {
    let mut fx = ChainFixture::build_unbound("preferences-no-root");
    assert!(
        matches!(
            dispatch_line("/preferences list", &mut fx),
            Dispatch::Handled
        ),
        "the refusal is inline"
    );
    let error =
        last_block(&fx.app.transcript, BlockKind::Error).expect("the refusal is an error block");
    assert!(
        error.contains("no project worktree here"),
        "the shared worktree remedy, verbatim: {error}"
    );
}

/// `/preferences bogus` is a parse error block naming the offence and
/// every valid form.
#[test]
fn preferences_dispatch_reports_an_unknown_subcommand() {
    let mut fx = ChainFixture::build("preferences-bogus");
    assert!(
        matches!(
            dispatch_line("/preferences bogus", &mut fx),
            Dispatch::Handled
        ),
        "the parse error is inline"
    );
    let error =
        last_block(&fx.app.transcript, BlockKind::Error).expect("a parse error is an error block");
    assert!(
        error.contains("unknown /preferences subcommand: bogus")
            && error.contains("/preferences forget-all"),
        "the error names the offence and the usage: {error}"
    );
}

/// The rooted success path through the real dispatch: `add` confirms one
/// preference and `list` shows it back with its `pp-` id — the same shared
/// operations the headless `saya preferences` runs, scoped to the session's
/// bound root. The fixture's default state db is the empty-path in-memory
/// store (per-connection databases), so this test points the App at a
/// file-backed store under the fixture root — production semantics — before
/// dispatching.
#[test]
fn preferences_add_and_list_through_dispatch() {
    let mut fx = ChainFixture::build("preferences-add-list");
    fx.app.state_db = saya_store::SqliteStateStore::new(fx.root.0.join("state.sqlite3"));
    assert!(
        matches!(
            dispatch_line("/preferences add prefer tabs over spaces", &mut fx),
            Dispatch::Handled
        ),
        "the add is inline"
    );
    let added = last_block(&fx.app.transcript, BlockKind::System).expect("the add says something");
    assert!(added.contains("Added preference pp-"), "{added}");
    assert!(
        matches!(
            dispatch_line("/preferences list", &mut fx),
            Dispatch::Handled
        ),
        "the listing is inline"
    );
    let listing = last_block(&fx.app.transcript, BlockKind::System).expect("the listing is said");
    assert!(listing.contains("prefer tabs over spaces"), "{listing}");
    assert!(
        listing.contains("pp-"),
        "the listing carries the id: {listing}"
    );
}

/// A symlinked AGENTS.md is refused by the loader, and `/context` says so by
/// name and reason — never printing the target's text.
#[cfg(unix)]
#[test]
fn context_dispatch_names_a_skipped_symlink_source() {
    use std::os::unix::fs::symlink;
    let mut fx = ChainFixture::build("context-symlink");
    let outside = fx.root.0.join("outside-agents.md");
    std::fs::write(&outside, "secret rules outside the workspace\n").unwrap();
    symlink(&outside, fx.root.0.join("AGENTS.md")).unwrap();
    assert!(
        matches!(dispatch_line("/context", &mut fx), Dispatch::Handled),
        "/context is handled by the dispatch chain"
    );
    let view =
        last_block(&fx.app.transcript, BlockKind::System).expect("/context pushes a system block");
    assert!(
        view.contains("AGENTS.md"),
        "the skipped source is named: {view}"
    );
    assert!(
        view.contains("symlink refused"),
        "the skip reason is shown: {view}"
    );
    assert!(
        !view.contains("secret rules"),
        "no text from the refused target leaks: {view}"
    );
}
