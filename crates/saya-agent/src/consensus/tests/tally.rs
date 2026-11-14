use super::support::{candidate, result};
use crate::consensus::{Consensus, tally};
use serde_json::json;

// ---------------------------------------------------------------------------
// tally behaviour
// ---------------------------------------------------------------------------

#[test]
fn tally_empty_input_does_not_panic() {
    let c = tally(&[], false);
    assert_eq!(
        c,
        Consensus {
            winner: None,
            votes: 0,
            margin: 0,
            tied: false,
            failed: 0,
        }
    );
}

#[test]
fn tally_all_failed() {
    let cands = vec![candidate("a", None), candidate("b", None)];
    let c = tally(&cands, false);
    assert_eq!(
        c,
        Consensus {
            winner: None,
            votes: 0,
            margin: 0,
            tied: false,
            failed: 2,
        }
    );
}

#[test]
fn tally_single_successful_wins() {
    let cands = vec![candidate("a", Some(result(&["v"], vec![json!([1])])))];
    let c = tally(&cands, false);
    assert_eq!(
        c,
        Consensus {
            winner: Some(0),
            votes: 1,
            margin: 1,
            tied: false,
            failed: 0,
        }
    );
}

#[test]
fn tally_failed_do_not_vote_but_are_counted() {
    let cands = vec![
        candidate("fail", None),
        candidate("ok", Some(result(&["v"], vec![json!([1])]))),
        candidate("fail2", None),
    ];
    let c = tally(&cands, false);
    assert_eq!(
        c,
        Consensus {
            winner: Some(1),
            votes: 1,
            margin: 1,
            tied: false,
            failed: 2,
        }
    );
}

#[test]
fn tally_largest_group_wins() {
    let cands = vec![
        candidate("a", Some(result(&["v"], vec![json!([1])]))),
        candidate("b", Some(result(&["v"], vec![json!([1])]))),
        candidate("c", Some(result(&["v"], vec![json!([2])]))),
    ];
    let c = tally(&cands, false);
    assert_eq!(
        c,
        Consensus {
            winner: Some(0),
            votes: 2,
            margin: 1,
            tied: false,
            failed: 0,
        }
    );
}

#[test]
fn tally_winner_is_first_member_in_input_order() {
    let cands = vec![
        candidate("c", Some(result(&["v"], vec![json!([2])]))),
        candidate("a", Some(result(&["v"], vec![json!([1])]))),
        candidate("b", Some(result(&["v"], vec![json!([1])]))),
    ];
    let c = tally(&cands, false);
    // Group {[1]} = indices [1, 2]; first member is 1.
    assert_eq!(
        c,
        Consensus {
            winner: Some(1),
            votes: 2,
            margin: 1,
            tied: false,
            failed: 0,
        }
    );
}

#[test]
fn tally_tie_for_lead_is_not_a_win() {
    let cands = vec![
        candidate("a", Some(result(&["v"], vec![json!([1])]))),
        candidate("b", Some(result(&["v"], vec![json!([2])]))),
    ];
    let c = tally(&cands, false);
    assert_eq!(
        c,
        Consensus {
            winner: None,
            votes: 1,
            margin: 0,
            tied: true,
            failed: 0,
        }
    );
}

#[test]
fn tally_three_way_tie() {
    let cands = vec![
        candidate("a", Some(result(&["v"], vec![json!([1])]))),
        candidate("b", Some(result(&["v"], vec![json!([2])]))),
        candidate("c", Some(result(&["v"], vec![json!([3])]))),
    ];
    let c = tally(&cands, false);
    assert_eq!(
        c,
        Consensus {
            winner: None,
            votes: 1,
            margin: 0,
            tied: true,
            failed: 0,
        }
    );
}

#[test]
fn tally_two_tied_groups_of_two() {
    let cands = vec![
        candidate("a", Some(result(&["v"], vec![json!([1])]))),
        candidate("b", Some(result(&["v"], vec![json!([1])]))),
        candidate("c", Some(result(&["v"], vec![json!([2])]))),
        candidate("d", Some(result(&["v"], vec![json!([2])]))),
    ];
    let c = tally(&cands, false);
    assert_eq!(
        c,
        Consensus {
            winner: None,
            votes: 2,
            margin: 0,
            tied: true,
            failed: 0,
        }
    );
}

#[test]
fn tally_ordered_passes_through_to_fingerprint() {
    let cands = vec![
        candidate("a", Some(result(&["v"], vec![json!([1]), json!([2])]))),
        candidate("b", Some(result(&["v"], vec![json!([2]), json!([1])]))),
    ];
    // Unordered: same rows -> agreement, winner is first member (index 0).
    let cu = tally(&cands, false);
    assert_eq!(
        cu,
        Consensus {
            winner: Some(0),
            votes: 2,
            margin: 2,
            tied: false,
            failed: 0,
        }
    );
    // Ordered: row order differs -> two singleton groups -> tie.
    let co = tally(&cands, true);
    assert_eq!(
        co,
        Consensus {
            winner: None,
            votes: 1,
            margin: 0,
            tied: true,
            failed: 0,
        }
    );
}
