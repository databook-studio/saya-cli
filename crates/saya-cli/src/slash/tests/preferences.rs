/// `/preferences` grammar: every form parses onto the same
/// `PreferencesCommand` the headless `saya preferences` parser
/// produces — no second DTO — and bare `/preferences` lists.
#[test]
fn test_parse_preferences_forms() {
    assert_eq!(
        parse_slash_command("/preferences"),
        Ok(Some(SlashCommand::Preferences(PreferencesCommand::List)))
    );
    assert_eq!(
        parse_slash_command("/preferences list"),
        Ok(Some(SlashCommand::Preferences(PreferencesCommand::List)))
    );
    assert_eq!(
        parse_slash_command("/preferences add prefer tabs over spaces"),
        Ok(Some(SlashCommand::Preferences(PreferencesCommand::Add {
            text: "prefer tabs over spaces".into()
        })))
    );
    assert_eq!(
        parse_slash_command("/preferences edit pp-1 new text here"),
        Ok(Some(SlashCommand::Preferences(PreferencesCommand::Edit {
            id: "pp-1".into(),
            text: "new text here".into()
        })))
    );
    assert_eq!(
        parse_slash_command("/preferences forget pp-1"),
        Ok(Some(SlashCommand::Preferences(
            PreferencesCommand::Forget { id: "pp-1".into() }
        )))
    );
    assert_eq!(
        parse_slash_command("/preferences forget-all"),
        Ok(Some(SlashCommand::Preferences(
            PreferencesCommand::ForgetAll
        )))
    );
}

/// Preference text is free text: the tail keeps its interior whitespace
/// verbatim — the `/sql` idiom, not the whitespace-collapsing default —
/// so a preference is stored exactly as typed.
#[test]
fn preferences_keeps_interior_spacing_verbatim() {
    assert_eq!(
        parse_slash_command("/preferences add  a  b"),
        Ok(Some(SlashCommand::Preferences(PreferencesCommand::Add {
            text: "a  b".into()
        })))
    );
    assert_eq!(
        parse_slash_command("/preferences edit pp-1  keep  this"),
        Ok(Some(SlashCommand::Preferences(PreferencesCommand::Edit {
            id: "pp-1".into(),
            text: "keep  this".into()
        })))
    );
}

/// Every malformed form is a usage error naming its remedy; the
/// unknown-subcommand error lists every valid form.
#[test]
fn preferences_malformed_forms_are_usage_errors() {
    let add_empty = parse_slash_command("/preferences add").unwrap_err();
    assert!(
        add_empty.0.contains("preferences add requires text"),
        "{add_empty}"
    );
    let edit_empty = parse_slash_command("/preferences edit").unwrap_err();
    assert!(
        edit_empty
            .0
            .contains("preferences edit requires an id and text"),
        "{edit_empty}"
    );
    let edit_no_text = parse_slash_command("/preferences edit pp-1").unwrap_err();
    assert!(
        edit_no_text
            .0
            .contains("preferences edit requires an id and text"),
        "{edit_no_text}"
    );
    let forget_empty = parse_slash_command("/preferences forget").unwrap_err();
    assert!(
        forget_empty.0.contains("preferences forget requires an id"),
        "{forget_empty}"
    );
    let forget_two = parse_slash_command("/preferences forget a b").unwrap_err();
    assert!(forget_two.0.contains("exactly one id"), "{forget_two}");
    let list_extra = parse_slash_command("/preferences list foo").unwrap_err();
    assert!(list_extra.0.contains("takes no argument"), "{list_extra}");
    let forget_all_extra = parse_slash_command("/preferences forget-all now").unwrap_err();
    assert!(
        forget_all_extra.0.contains("forget-all takes no argument"),
        "{forget_all_extra}"
    );
    let unknown = parse_slash_command("/preferences bogus").unwrap_err();
    assert!(
        unknown.0.contains("unknown /preferences subcommand: bogus")
            && unknown.0.contains("/preferences forget-all"),
        "the unknown-subcommand error names every valid form: {unknown}"
    );
}
