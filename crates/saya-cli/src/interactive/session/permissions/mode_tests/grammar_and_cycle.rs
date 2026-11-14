use saya_agent::PermissionMode;

use super::super::{BYPASS_REFUSAL, name, next_in_cycle, parse, stated_mode};

/// `/mode`'s grammar: the four modes, and `build` kept as an alias for
/// `ask` — the spelling the task-posture command used before the two axes
/// became one mode. Case-sensitive like `/approvals`, and an unknown word is
/// nothing, never a silent default.
#[test]
fn the_grammar_names_four_modes_and_keeps_build_as_an_alias() {
    assert_eq!(parse("ask"), Some(PermissionMode::Ask));
    assert_eq!(
        parse("build"),
        Some(PermissionMode::Ask),
        "`build` is the old spelling of the same posture"
    );
    assert_eq!(parse("accept-edits"), Some(PermissionMode::AcceptEdits));
    assert_eq!(parse("plan"), Some(PermissionMode::Plan));
    assert_eq!(parse("bypass"), Some(PermissionMode::Bypass));
    assert_eq!(parse("Plan"), None, "the parse is case-sensitive");
    assert_eq!(parse("nonsense"), None);
    assert_eq!(parse(""), None);
    assert_eq!(name(PermissionMode::AcceptEdits), "accept-edits");
}

/// The keyboard cycle walks `ask → accept-edits → plan → bypass → ask` when
/// the configuration allows bypass, and skips it entirely when it does not:
/// a key that could reach the mode would be a switch `/mode` refuses.
#[test]
fn the_cycle_skips_bypass_when_the_config_withholds_it() {
    assert_eq!(
        next_in_cycle(PermissionMode::Ask, false),
        PermissionMode::AcceptEdits
    );
    assert_eq!(
        next_in_cycle(PermissionMode::AcceptEdits, false),
        PermissionMode::Plan
    );
    assert_eq!(
        next_in_cycle(PermissionMode::Plan, false),
        PermissionMode::Ask,
        "bypass is skipped, never offered"
    );
    assert_eq!(
        next_in_cycle(PermissionMode::Ask, true),
        PermissionMode::AcceptEdits
    );
    assert_eq!(
        next_in_cycle(PermissionMode::Plan, true),
        PermissionMode::Bypass
    );
    assert_eq!(
        next_in_cycle(PermissionMode::Bypass, true),
        PermissionMode::Ask,
        "the cycle closes"
    );
    assert_eq!(
        next_in_cycle(PermissionMode::Bypass, false),
        PermissionMode::Ask,
        "a session already under bypass still leaves it"
    );
}

/// A launch's `--permission-mode` reads through the same grammar the command
/// does, and its refusals are the command's own wording: an unknown word is
/// named with the four it accepts, and `bypass` answers the switch's refusal
/// rather than inventing a second sentence for the same request.
#[test]
fn a_stated_launch_mode_parses_and_refuses_like_the_command() {
    assert_eq!(
        stated_mode("accept-edits", false),
        Ok(PermissionMode::AcceptEdits)
    );
    assert_eq!(
        stated_mode("build", false),
        Ok(PermissionMode::Ask),
        "the alias the command keeps is the alias the flag keeps"
    );
    let error = stated_mode("Plan", false).expect_err("the parse is case-sensitive");
    assert!(
        error.contains("accept-edits") && error.contains("bypass"),
        "an unknown word names the modes that parse: {error}"
    );
    assert_eq!(
        stated_mode("bypass", false),
        Err(BYPASS_REFUSAL.to_owned()),
        "a withheld bypass answers with the switch's own refusal"
    );
    assert_eq!(
        stated_mode("bypass", true),
        Ok(PermissionMode::Bypass),
        "the key the switch reads is the key the flag reads"
    );
}
