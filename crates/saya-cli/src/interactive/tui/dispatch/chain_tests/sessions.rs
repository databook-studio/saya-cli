/// `/sessions list` through the real dispatch: the shared listing the
/// headless `saya sessions list` prints lands as a System block — this
/// fixture's store is empty, so it says so and names the storage root.
#[test]
fn sessions_list_dispatch_shares_the_headless_listing() {
    let mut fx = ChainFixture::build("sessions-list");
    let outcome = dispatch_line("/sessions list", &mut fx);
    assert!(
        matches!(outcome, Dispatch::Handled),
        "the listing is handled inline"
    );
    let listing =
        last_block(&fx.app.transcript, BlockKind::System).expect("/sessions list pushes a block");
    assert!(
        listing.starts_with("No saved sessions.") || listing.contains("Storage root:"),
        "the listing says what the store holds and names the root: {listing}"
    );
}

/// `/sessions delete <the session's own id>` through the real dispatch: the
/// current-session refusal, the D09a words, nothing deleted — the check
/// fires before any filesystem work, so no tombstone is ever written.
#[test]
fn sessions_delete_dispatch_refuses_the_current_session() {
    let mut fx = ChainFixture::build("sessions-delete-current");
    let id = fx.state.id.clone();
    let outcome = dispatch_line(&format!("/sessions delete {id}"), &mut fx);
    assert!(
        matches!(outcome, Dispatch::Handled),
        "the refusal is inline"
    );
    let error =
        last_block(&fx.app.transcript, BlockKind::Error).expect("the refusal is an error block");
    assert!(
        error.contains("live session") && error.contains("was not deleted"),
        "the refusal is the current-session one, never a silent no-op: {error}"
    );
    assert!(
        !error.contains("pid"),
        "the current-session refusal never reads as a foreign live holder: {error}"
    );
    assert!(
        !fx.root.0.join("lifecycle").exists(),
        "no tombstone was written for the current session"
    );
}

/// `/sessions delete <unknown id>` through the real dispatch: the refusal
/// names the id and nothing is created.
#[test]
fn sessions_delete_dispatch_refuses_an_unknown_id() {
    let mut fx = ChainFixture::build("sessions-delete-nosuch");
    let outcome = dispatch_line("/sessions delete nosuchid-0000", &mut fx);
    assert!(
        matches!(outcome, Dispatch::Handled),
        "the refusal is inline"
    );
    let error =
        last_block(&fx.app.transcript, BlockKind::Error).expect("the refusal is an error block");
    assert!(
        error.contains("no saved session nosuchid-0000"),
        "the refusal names the unknown id: {error}"
    );
    assert!(
        !fx.root.0.join("lifecycle").exists(),
        "an unknown id writes no tombstone"
    );
}

/// `/sessions delete ../x` through the real dispatch: the invalid-id
/// refusal, with nothing resolved on the filesystem.
#[test]
fn sessions_delete_dispatch_refuses_an_invalid_id() {
    let mut fx = ChainFixture::build("sessions-delete-invalid");
    let outcome = dispatch_line("/sessions delete ../x", &mut fx);
    assert!(
        matches!(outcome, Dispatch::Handled),
        "the refusal is inline"
    );
    let error =
        last_block(&fx.app.transcript, BlockKind::Error).expect("the refusal is an error block");
    assert!(
        error.contains("not a valid session id"),
        "the refusal is the invalid-id one: {error}"
    );
}
