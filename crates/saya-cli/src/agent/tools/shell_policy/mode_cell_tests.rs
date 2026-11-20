//! The job's mode cell read through a capability cap: a capped view follows the
//! cell it was taken from, and never resolves wider than the cap.

use saya_agent::ApprovalPolicy;

use super::ModeCell;

/// A capped view is the same cell: a mode stated on the job's cell is the mode
/// the view resolves at its next read, so a switch narrows a live caller.
#[test]
fn a_capped_view_follows_the_cell_it_was_taken_from() {
    let cell = ModeCell::new(ApprovalPolicy::Ask);
    let read_only = cell.capped_at(ApprovalPolicy::ReadOnly);

    assert_eq!(read_only.get(), ApprovalPolicy::ReadOnly);
    cell.set(ApprovalPolicy::ReadOnly)
        .expect("an unfrozen cell takes a change");
    assert_eq!(
        read_only.get(),
        ApprovalPolicy::ReadOnly,
        "the view reads the stated mode"
    );
    cell.set(ApprovalPolicy::Never)
        .expect("an unfrozen cell takes a change");
    assert_eq!(
        read_only.get(),
        ApprovalPolicy::Never,
        "a mode narrower than the cap is resolved as stated, not widened to it"
    );
}

/// The cap is a ceiling and nothing more: whatever the cell states, a read-only
/// capability never resolves a mode that would reach the host, and a cap that
/// narrows nothing leaves the cell's own answers alone.
#[test]
fn a_cap_is_a_ceiling_over_every_mode_the_cell_can_state() {
    let cell = ModeCell::new(ApprovalPolicy::Ask);
    let read_only = cell.capped_at(ApprovalPolicy::ReadOnly);
    for stated in [
        ApprovalPolicy::Ask,
        ApprovalPolicy::AcceptEdits,
        ApprovalPolicy::Bypass,
    ] {
        cell.set(stated).expect("an unfrozen cell takes a change");
        assert_eq!(
            read_only.get(),
            ApprovalPolicy::ReadOnly,
            "a read-only capability holds no wider mode than read-only, not {stated:?}"
        );
    }

    let uncapped = cell.capped_at(ApprovalPolicy::Bypass);
    for stated in [
        ApprovalPolicy::ReadOnly,
        ApprovalPolicy::Ask,
        ApprovalPolicy::Bypass,
    ] {
        cell.set(stated).expect("an unfrozen cell takes a change");
        assert_eq!(
            uncapped.get(),
            stated,
            "a cap that narrows nothing resolves the cell's own mode"
        );
    }
}

/// A capped view of a frozen cell refuses a change and reports it, exactly as
/// the cell it was taken from does: the cap narrows a mode, it never unlocks
/// one.
#[test]
fn a_capped_view_of_a_frozen_cell_still_refuses_a_change() {
    let cell = ModeCell::new(ApprovalPolicy::ReadOnly);
    cell.freeze_at(ApprovalPolicy::ReadOnly);
    let capped = cell.capped_at(ApprovalPolicy::ReadOnly);

    assert!(capped.is_frozen(), "the view reads the cell's own freeze");
    assert!(
        capped.set(ApprovalPolicy::Bypass).is_err(),
        "a frozen cell refuses through the view too"
    );
    assert_eq!(cell.get(), ApprovalPolicy::ReadOnly);
    assert_eq!(capped.get(), ApprovalPolicy::ReadOnly);
}
