use super::*;

/// A successor's carried summary (`compacted_turns == 0`) replays under a
/// local provider: it reaches `provider_history()[0]` with the carried-over
/// header, since its source turns cannot be re-checked here.
#[test]
fn a_successor_replays_its_carried_summary_under_a_local_provider() {
    let parent = compacted_parent();
    let successor = rotate_state(&parent, "s2", "rot-1").expect("a compacted parent rotates");
    assert_eq!(successor.compacted_turns, 0);
    let history = successor.provider_history();
    let first = history.first().expect("the carried summary leads");
    assert_eq!(first.role, "user");
    assert!(first.content.contains(SUMMARY));
    assert!(
        first
            .content
            .contains("carried over from the previous session")
    );
}

/// The same successor under a cloud provider with sharing refused withholds
/// the carried summary everywhere: it is potentially database-derived and
/// its source turns cannot be re-checked.
#[test]
fn a_successor_withholds_its_carried_summary_when_sharing_is_refused() {
    let parent = compacted_parent();
    let mut successor = rotate_state(&parent, "s2", "rot-1").expect("a compacted parent rotates");
    successor.provider = "openai".into();
    successor.allow_data_sharing = false;
    assert!(successor.provider_endpoint.is_none());
    assert!(
        successor.compaction_summary_text().is_none(),
        "the raw summary is gated off"
    );
    let history = successor.provider_history();
    assert!(
        history
            .iter()
            .all(|message| !message.content.contains(SUMMARY)),
        "no summary entry rides into the request"
    );
    assert_eq!(
        history.first().map(|message| message.content.as_str()),
        Some("q4"),
        "the verbatim tail still leads the request"
    );
}

/// Explicit sharing on the same cloud successor admits the carried summary.
#[test]
fn a_successor_replays_its_carried_summary_when_sharing_is_allowed() {
    let parent = compacted_parent();
    let mut successor = rotate_state(&parent, "s2", "rot-1").expect("a compacted parent rotates");
    successor.provider = "openai".into();
    successor.allow_data_sharing = true;
    let history = successor.provider_history();
    let first = history.first().expect("the carried summary leads");
    assert!(first.content.contains(SUMMARY));
    assert!(
        first
            .content
            .contains("carried over from the previous session")
    );
}

/// A normally compacted state (`compacted_turns > 0`) keeps its original
/// gate behavior in both privacy postures: withheld only when a
/// database-derived turn exists under refused sharing.
#[test]
fn a_normally_compacted_state_keeps_its_original_gate_behavior() {
    let mut local = turns(6);
    compact(&mut local, "kept summary");
    let compacted_turns = local.compacted_turns;
    assert!(compacted_turns > 0, "fixture sanity: a real compaction ran");
    local.provider = "ollama".into();
    local.allow_data_sharing = false;
    local.provider_endpoint = None;
    assert!(local.compaction_summary_text().is_some());
    assert!(
        local
            .compaction_narrative()
            .unwrap()
            .contains(&format!("compacted from {compacted_turns} turns"))
    );

    let mut cloud = turns(6);
    compact(&mut cloud, "kept summary");
    cloud.provider = "openai".into();
    cloud.allow_data_sharing = false;
    cloud.provider_endpoint = None;
    cloud.record_turn("q3", "row answer", true, Vec::new());
    assert!(cloud.compaction_summary_text().is_none());
    assert!(cloud.compaction_narrative().is_none());

    cloud.allow_data_sharing = true;
    assert!(cloud.compaction_summary_text().is_some());
}
