// K2: `/mode` is the one consent axis and `/approvals` is its one-release
// alias. These tests pin the parse: six values on `/mode`, the alias landing on
// the *same* variant carrying the spelling that stated it, and an unknown word
// naming the whole vocabulary.

use saya_agent::PermissionMode;

/// One statement of a consent value, through the canonical spelling: the same
/// variant `/approvals` produces, and the note is not owed.
fn mode(value: Option<PermissionMode>) -> SlashCommand {
    SlashCommand::Mode(ModeRequest {
        value,
        ..Default::default()
    })
}

/// The alias statement of the same value: one variant, one grammar, and the
/// request remembers that `/approvals` said it.
fn alias(value: Option<PermissionMode>) -> SlashCommand {
    SlashCommand::Mode(ModeRequest {
        value,
        via_approvals_alias: true,
    })
}

/// `/mode` accepts all six values. `read-only` and `never` used to live on
/// `/approvals` alone; one axis means the mode command names them.
#[test]
fn mode_states_all_six_values() {
    for (spelling, expected) in [
        ("ask", PermissionMode::Ask),
        ("accept-edits", PermissionMode::AcceptEdits),
        ("plan", PermissionMode::Plan),
        ("bypass", PermissionMode::Bypass),
        ("read-only", PermissionMode::ReadOnly),
        ("never", PermissionMode::Never),
    ] {
        assert_eq!(
            parse_slash_command(&format!("/mode {spelling}")),
            Ok(Some(mode(Some(expected)))),
            "/mode {spelling} must state {expected:?}"
        );
    }
    assert_eq!(parse_slash_command("/mode"), Ok(Some(mode(None))));
}

/// `/approvals <value>` is the same command: the same request, the same value,
/// and the one flag that tells the surface which spelling asked.
#[test]
fn approvals_is_the_one_release_alias_of_mode() {
    for (spelling, expected) in [
        ("ask", PermissionMode::Ask),
        ("accept-edits", PermissionMode::AcceptEdits),
        ("plan", PermissionMode::Plan),
        ("bypass", PermissionMode::Bypass),
        ("read-only", PermissionMode::ReadOnly),
        ("never", PermissionMode::Never),
    ] {
        assert_eq!(
            parse_slash_command(&format!("/approvals {spelling}")),
            Ok(Some(alias(Some(expected)))),
            "/approvals {spelling} must state the same {expected:?}"
        );
    }
    assert_eq!(
        parse_slash_command("/approvals"),
        Ok(Some(alias(None))),
        "a bare alias reads the mode back like a bare /mode"
    );
}

/// An unknown value names every value that parses, on both spellings — a
/// refusal that silently defaulted would be a consent nobody stated.
#[test]
fn an_unknown_value_names_the_six_values_on_both_spellings() {
    for name in ["/mode", "/approvals"] {
        let error = parse_slash_command(&format!("{name} sometimes")).unwrap_err();
        for value in ["ask", "accept-edits", "plan", "bypass", "read-only", "never"] {
            assert!(
                error.0.contains(value),
                "{name}'s refusal must name `{value}`, got: {}",
                error.0
            );
        }
    }
    assert_eq!(
        parse_slash_command("/mode Plan").unwrap_err().0,
        "invalid permission mode: Plan (expected ask, accept-edits, plan, bypass, read-only, \
         or never)",
        "the parse stays case-sensitive"
    );
}
