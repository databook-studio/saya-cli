//! The toast (`02` §4): said once, lived four seconds of ticks, replaced by the
//! next one. Driven through the update, because the tick is a message the model
//! folds rather than a clock it reads.

use std::time::Duration;

use super::super::model::Model;
use super::super::msg::{Msg, Worker};
use super::super::update::update;
use super::toast::{LIFETIME, TICK, TICKS, Toast};

/// Sends `count` ticks, as the runtime does, and answers nothing.
fn ticks(model: &mut Model, count: u32) {
    for _ in 0..count {
        assert!(
            update(model, Msg::Tick).is_empty(),
            "a tick asks the runtime for nothing"
        );
    }
}

/// How many ticks fit in 3.9 seconds: the last tick before the toast's end.
const ALMOST: u32 = 15;

#[test]
fn the_toast_lifetime_is_four_seconds_of_the_tick() {
    assert_eq!(TICK, Duration::from_millis(250), "the tick T3 defines");
    assert_eq!(TICK * TICKS, LIFETIME, "the life is sixteen ticks");
    assert_eq!(LIFETIME, Duration::from_secs(4), "`02` §4's four seconds");
    assert!(
        TICK * ALMOST < Duration::from_millis(3_900),
        "fifteen ticks are still inside 3.9 s"
    );
    assert!(
        TICK * TICKS > Duration::from_millis(3_900),
        "and sixteen are past it"
    );
}

#[test]
fn a_toast_is_still_said_after_three_point_nine_seconds_of_ticks() {
    let mut model = Model::default();
    model.say("copied answer (1.2 kB)");
    assert_eq!(model.toast(), Some("copied answer (1.2 kB)"));

    ticks(&mut model, ALMOST);

    assert_eq!(
        model.toast(),
        Some("copied answer (1.2 kB)"),
        "3.75 s of ticks is inside the toast's four seconds"
    );
    assert_eq!(
        model.toast_value().map(Toast::remaining),
        Some(TICKS - ALMOST),
        "the clock counts down a tick at a time"
    );
}

#[test]
fn a_toast_is_gone_after_four_seconds_of_ticks() {
    let mut model = Model::default();
    update(
        &mut model,
        Msg::Worker(Worker {
            message: "the export could not be written".to_owned(),
        }),
    );
    assert!(model.toast().is_some(), "a worker's word is said");

    ticks(&mut model, TICKS);

    assert_eq!(
        model.toast(),
        None,
        "four seconds of ticks is the whole life of a toast"
    );
    assert!(
        model.toast_value().is_none(),
        "an expired toast is not kept behind the words"
    );
}

#[test]
fn a_second_toast_replaces_the_first_and_restarts_the_clock() {
    let mut model = Model::default();
    model.say("mode → plan: write tools refuse until you leave plan");
    ticks(&mut model, 10);

    update(
        &mut model,
        Msg::Worker(Worker {
            message: "copied answer (1.2 kB)".to_owned(),
        }),
    );

    assert_eq!(
        model.toast(),
        Some("copied answer (1.2 kB)"),
        "the newer words replace the older ones"
    );
    assert_eq!(
        model.toast_value().map(Toast::remaining),
        Some(TICKS),
        "a new toast owns the whole four seconds, not what the last one left"
    );

    ticks(&mut model, ALMOST);
    assert_eq!(
        model.toast(),
        Some("copied answer (1.2 kB)"),
        "the clock restarted with the second toast"
    );

    ticks(&mut model, 1);
    assert_eq!(
        model.toast(),
        None,
        "and it still expires four seconds after it was said"
    );
}

#[test]
fn a_tick_with_no_toast_changes_nothing() {
    let mut model = Model::default();
    ticks(&mut model, TICKS + 1);
    assert_eq!(model.toast(), None, "nothing was said, nothing expires");
    assert_eq!(model.composer(), "", "a tick never types for the operator");
}
