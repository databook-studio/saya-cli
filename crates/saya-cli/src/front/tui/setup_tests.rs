//! What a closed guided setup means on this surface.
//!
//! The old surface rendered these outcomes onto its `App` (`setup_apply.rs`,
//! deleted with the trunk). The meaning — which ending applies the new
//! configuration, which changes nothing, and what each says — must survive
//! with the surface, so these pin [`landed`] instead: the same endings, the
//! same words, and the one ending that must never reach the screen (a lost
//! terminal).

use std::io;

use super::ops::Asked;
use super::setup::landed;
use crate::interactive::tui::SetupHandoff;

/// The words an ending asks for, or a panic naming what came back instead.
fn words(asked: Asked) -> String {
    match asked {
        Asked::Notice(text) | Asked::Error(text) => text,
        other => panic!("the setup close is said in words, not {other:?}"),
    }
}

/// A reload that was asked for: its text, and whether it ran at all.
fn reload<'a>(count: &'a std::cell::Cell<usize>, asked: Asked) -> impl FnOnce() -> Asked + 'a {
    move || {
        count.set(count.get() + 1);
        asked
    }
}

/// `Finished(0)` is also the wizard's own "cancelled, nothing written" exit:
/// it still runs the same reload `/reload` runs, and ONE notice carries the
/// close plus the reload's unchanged line — never two blocks.
#[test]
fn a_completed_setup_applies_the_reload_and_says_closed() {
    let count = std::cell::Cell::new(0);
    let said = words(
        landed(
            Ok(SetupHandoff::Finished(0)),
            reload(
                &count,
                Asked::Notice(String::from("Configuration is unchanged.")),
            ),
        )
        .expect("a closed wizard lands"),
    );
    assert_eq!(
        count.get(),
        1,
        "a completed setup applies the reload exactly once"
    );
    assert_eq!(
        said, "Guided setup closed. Configuration is unchanged.",
        "one line names the close and the reload's own words"
    );
}

/// A reload that refused keeps its words and says the configuration was NOT
/// applied: the surface must never claim an apply that did not happen.
#[test]
fn a_failed_reload_says_the_configuration_could_not_be_applied() {
    let count = std::cell::Cell::new(0);
    let said = words(
        landed(
            Ok(SetupHandoff::Finished(0)),
            reload(
                &count,
                Asked::Error(String::from("connection 'x' is unknown")),
            ),
        )
        .expect("a closed wizard lands"),
    );
    assert_eq!(count.get(), 1);
    assert_eq!(
        said,
        "Guided setup closed, but the new configuration could not be applied: \
         connection 'x' is unknown",
        "the failure names what was not applied, in the reload's own words"
    );
}

/// A non-zero exit is the wizard telling us it wrote something it cannot
/// vouch for: a refusal naming the code, and no reload at all.
#[test]
fn a_non_zero_exit_refuses_and_never_reloads() {
    let count = std::cell::Cell::new(0);
    let said = words(
        landed(Ok(SetupHandoff::Finished(2)), reload(&count, Asked::Reload))
            .expect("a closed wizard lands"),
    );
    assert_eq!(
        count.get(),
        0,
        "nothing is applied that the wizard did not report"
    );
    assert_eq!(
        said,
        "Guided setup ended with exit code 2. Nothing was applied that the \
         wizard did not report."
    );
}

/// A wizard that failed carries its own rendered error text, and reloads
/// nothing.
#[test]
fn a_failed_wizard_states_its_own_error_and_never_reloads() {
    let count = std::cell::Cell::new(0);
    let said = words(
        landed(
            Ok(SetupHandoff::Failed(String::from("probe timed out"))),
            reload(&count, Asked::Reload),
        )
        .expect("a closed wizard lands"),
    );
    assert_eq!(count.get(), 0);
    assert_eq!(said, "Guided setup failed: probe timed out");
}

/// A lost terminal is returned, never rendered: the caller must stop drawing
/// into a terminal it no longer owns.
#[test]
fn a_lost_terminal_propagates_without_words() {
    let count = std::cell::Cell::new(0);
    let error = landed(
        Err(io::Error::other("reacquire failed")),
        reload(&count, Asked::Notice(String::new())),
    )
    .expect_err("a lost terminal is an error, not a rendered outcome");
    assert!(error.to_string().contains("reacquire failed"), "{error}");
    assert_eq!(count.get(), 0, "a lost terminal applies nothing");
}
