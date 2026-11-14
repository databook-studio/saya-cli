use super::support::*;
use super::*;

/// A fresh launch derives the header facts from the recomposed universe —
/// lane composed where the workspace bound, deny list the launch stated —
/// and journals the deny list exactly once.
#[test]
fn a_fresh_launch_derives_the_header_facts_and_journals_the_deny_list() {
    let mut fx = Fixture::build_fresh("fresh");
    derive_launch_posture(&mut fx.state, &fx.session, true);
    assert_eq!(
        fx.state.host_composed,
        fx.session.universe().host_composed(),
        "the header's lane bit is the launch universe's fact"
    );
    assert!(
        fx.state.host_composed,
        "the explicit workspace bound a root: the lane composes"
    );
    assert_eq!(
        fx.state.denied_programs,
        fx.session.universe().deny_programs(),
        "the header's deny list is the launch universe's fact"
    );
    assert_eq!(fx.state.denied_programs, vec![DENIED.to_owned()]);
    assert_eq!(
        deny_journal_events(&fx.session),
        vec![vec![DENIED.to_owned()]],
        "a fresh start journals its deny list once"
    );
}
