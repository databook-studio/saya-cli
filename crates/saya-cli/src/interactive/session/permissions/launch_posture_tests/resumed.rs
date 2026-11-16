use super::support::*;
use super::*;

/// A resumed launch derives the header facts too: the acquired universe
/// composed the lane from the recorded pin, and its deny list (the user-layer
/// config's, folded in at composition) is what the header copies. The deny
/// journal writes nothing — the launch statement belonged to the previous
/// process, whose start journaled its own list.
#[test]
fn a_resumed_launch_derives_the_header_facts_from_its_composed_universe() {
    let root = root_for("resume-pin");
    let pin = project_dir(&root);
    let mut fx = Fixture::build_resumed("resume", Some(pin.clone()), &[DENIED]);
    derive_launch_posture(&mut fx.state, &fx.session, false);
    assert_eq!(
        fx.state.host_composed,
        fx.session.universe().host_composed(),
        "the header's lane bit is the resumed universe's fact"
    );
    assert!(
        fx.state.host_composed,
        "the recorded pin re-binds a root: the lane composes for the header"
    );
    assert_eq!(
        fx.state.denied_programs,
        fx.session.universe().deny_programs(),
        "the header's deny list is the resumed universe's fact"
    );
    assert_eq!(
        fx.state.denied_programs,
        vec![DENIED.to_owned()],
        "the config deny composed into the universe is what the header carries"
    );
    assert!(
        deny_journal_events(&fx.session).is_empty(),
        "a resumed launch journals no deny list"
    );
    drop(fx);
    let _ = std::fs::remove_dir_all(&root);
}
