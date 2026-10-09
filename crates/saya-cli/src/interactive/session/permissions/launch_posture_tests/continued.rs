use super::support::*;
use super::*;

/// A continued launch derives the header facts from the unbound shape too:
/// the lane did not compose and the header must say so (a stale `true` never
/// survives), while the deny list still rides the laneless universe.
#[test]
fn a_continued_launch_derives_the_header_facts_and_journals_nothing() {
    let mut fx = Fixture::build_continued("continue", None, &[DENIED]);
    fx.state.host_composed = true;
    derive_launch_posture(&mut fx.state, &fx.session, false);
    assert_eq!(
        fx.state.host_composed,
        fx.session.universe().host_composed(),
        "the header's lane bit is the continued universe's fact"
    );
    assert!(
        !fx.state.host_composed,
        "no root binds on this continued launch: the header says the lane is off"
    );
    assert_eq!(
        fx.state.denied_programs,
        fx.session.universe().deny_programs(),
        "the deny list rides even where the lane did not compose"
    );
    assert_eq!(fx.state.denied_programs, vec![DENIED.to_owned()]);
    assert!(
        deny_journal_events(&fx.session).is_empty(),
        "a continued launch journals no deny list"
    );
}
