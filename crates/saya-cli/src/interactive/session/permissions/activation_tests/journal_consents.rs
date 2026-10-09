/// The journal's bypass-activation decision (U7), as a property of the
/// system: the journal records consents, so it records exactly the moments
/// the user consented to the mode. A fresh session runs under the mode its
/// launch stated — bypass there is an activation. A resume whose
/// `--approval-mode` explicitly overrode the record is a new statement of
/// consent this process made. A resume that merely carries the persisted
/// mode re-prints the line but consents to nothing new — the record made
/// the mode operative — so nothing is journalled for it. An unparseable
/// mode is never bypass.
#[test]
fn the_launch_journals_bypass_exactly_when_this_process_consented_to_it() {
    use super::super::bypass_activated_at_launch;
    // A fresh session under the mode its launch stated.
    assert!(
        bypass_activated_at_launch(true, false, "bypass"),
        "a fresh session under bypass is an activation"
    );
    // A resume whose flag explicitly overrode the persisted mode.
    assert!(
        bypass_activated_at_launch(false, true, "bypass"),
        "an explicit `--approval-mode bypass` on a resume is a new consent"
    );
    // A resume that merely carries the persisted mode: the line re-prints,
    // but nothing new is consented to, so nothing is journalled.
    assert!(
        !bypass_activated_at_launch(false, false, "bypass"),
        "a carried bypass mode is the record's consent, not this process's"
    );
    // Any other mode journals nothing.
    for mode in ["ask", "read-only", "never", ""] {
        assert!(
            !bypass_activated_at_launch(true, false, mode),
            "no line for {mode:?}: only bypass activation is journalled"
        );
    }
}

/// The mid-session `/approvals bypass` journals a consent only when the
/// command newly activated the mode: the mode was not bypass before it and
/// is bypass after. A re-statement over an already-bypass session changes
/// nothing — like `/allow` over an already-granted token it says so, but
/// records no new consent — and a flip to a narrower mode is a narrowing,
/// never a widening, so it records none either.
#[test]
fn the_command_journals_bypass_only_when_it_newly_activates_the_mode() {
    use super::super::bypass_activated_by_command;
    // ask -> bypass: the consent the line announces.
    assert!(
        bypass_activated_by_command("ask", "bypass"),
        "the command that flips the mode is the activation it prints"
    );
    // bypass -> bypass: a re-statement. Nothing changed, nothing consented.
    assert!(
        !bypass_activated_by_command("bypass", "bypass"),
        "a re-statement over an already-bypass session records no new consent"
    );
    // bypass -> ask: a narrowing, not a widening.
    assert!(
        !bypass_activated_by_command("bypass", "ask"),
        "a narrowing mode change grants nothing"
    );
    // read-only -> bypass: still an activation — the widening is real.
    assert!(
        bypass_activated_by_command("read-only", "bypass"),
        "the widening is real whatever the mode it came from"
    );
}
