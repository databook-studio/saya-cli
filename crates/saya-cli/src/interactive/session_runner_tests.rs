//! `runner_grant_notice`'s pure cases: no composed runner, a composed
//! runner missing the program, and a composed runner carrying it.

use super::runner_grant_notice;

#[test]
fn no_composed_runner_names_the_config_gap() {
    let notice = runner_grant_notice("python3", None).expect("no runner composed: a notice is due");
    assert!(
        notice.contains("runner:python3 granted, but no runner is configured"),
        "the notice states the fact: {notice:?}"
    );
    assert!(
        notice.contains("[jobs.runner]") && notice.contains("program_dir"),
        "the notice names the remedy config keys: {notice:?}"
    );
    assert!(
        notice.contains("run_command") && notice.contains("ask for approval"),
        "the notice names the fallback: {notice:?}"
    );
}

#[test]
fn a_program_outside_the_composed_allow_names_the_gap() {
    let programs = vec!["node".to_owned()];
    let notice = runner_grant_notice("python3", Some(&programs))
        .expect("outside the composed allow: a notice is due");
    assert!(
        notice.contains("runner:python3 granted, but python3 is not in [jobs.runner] allow"),
        "the notice names the gap: {notice:?}"
    );
    assert!(
        notice.contains("run_command") && notice.contains("ask for approval"),
        "the notice names the fallback: {notice:?}"
    );
}

#[test]
fn a_program_inside_the_composed_allow_is_silent() {
    let programs = vec!["python3".to_owned()];
    assert!(
        runner_grant_notice("python3", Some(&programs)).is_none(),
        "the composed runner's door carries the program: nothing to say"
    );
}
