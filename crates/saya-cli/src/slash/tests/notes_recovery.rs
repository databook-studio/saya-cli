/// The `/notes` grammar: bare and `show` show, `set <text>` keeps the
/// text verbatim (outer ends trimmed, interior spacing intact), `clear`
/// takes nothing.
#[test]
fn notes_grammar_parses_every_form() {
    use crate::interactive::session::lifecycle::notes::NotesCommand;
    assert_eq!(
        parse_slash_command("/notes"),
        Ok(Some(SlashCommand::Notes(NotesCommand::Show)))
    );
    assert_eq!(
        parse_slash_command("/notes show"),
        Ok(Some(SlashCommand::Notes(NotesCommand::Show)))
    );
    assert_eq!(
        parse_slash_command("/notes clear"),
        Ok(Some(SlashCommand::Notes(NotesCommand::Clear)))
    );
    assert_eq!(
        parse_slash_command("/notes set remember the failing query"),
        Ok(Some(SlashCommand::Notes(NotesCommand::Set(
            "remember the failing query".into()
        ))))
    );
}

/// Notes text is free text: the tail keeps its interior whitespace
/// verbatim — the `/sql` idiom, like `/preferences` — and only the
/// outer ends are trimmed.
#[test]
fn notes_keeps_interior_spacing_verbatim() {
    use crate::interactive::session::lifecycle::notes::NotesCommand;
    assert_eq!(
        parse_slash_command("/notes set  a  b   c  "),
        Ok(Some(SlashCommand::Notes(NotesCommand::Set(
            "a  b   c".into()
        ))))
    );
}

/// Every malformed form is a usage error naming its remedy; the
/// unknown-subcommand error lists every valid form.
#[test]
fn notes_malformed_forms_are_usage_errors() {
    let set_empty = parse_slash_command("/notes set").unwrap_err();
    assert!(
        set_empty
            .0
            .contains("notes set requires text: /notes set <text>"),
        "{set_empty}"
    );
    let show_extra = parse_slash_command("/notes show extra").unwrap_err();
    assert!(show_extra.0.contains("takes no argument"), "{show_extra}");
    let clear_extra = parse_slash_command("/notes clear now").unwrap_err();
    assert!(clear_extra.0.contains("takes no argument"), "{clear_extra}");
    let unknown = parse_slash_command("/notes bogus").unwrap_err();
    assert!(
        unknown.0.contains("unknown /notes subcommand: bogus")
            && unknown.0.contains("/notes set <text>")
            && unknown.0.contains("/notes clear"),
        "the unknown-subcommand error names every valid form: {unknown}"
    );
}

/// `/recovery` parses every sub-word form: bare (or `status`) shows the
/// status, on/off flip the flag, list lists, and diff/restore carry the
/// typed id (full `ck-…` or a unique prefix) verbatim.
#[test]
fn recovery_grammar_parses_every_form() {
    use crate::commands::recovery::RecoveryCommand;
    assert_eq!(
        parse_slash_command("/recovery"),
        Ok(Some(SlashCommand::Recovery(RecoveryCommand::Status)))
    );
    assert_eq!(
        parse_slash_command("/recovery status"),
        Ok(Some(SlashCommand::Recovery(RecoveryCommand::Status)))
    );
    assert_eq!(
        parse_slash_command("/recovery on"),
        Ok(Some(SlashCommand::Recovery(RecoveryCommand::On)))
    );
    assert_eq!(
        parse_slash_command("/recovery off"),
        Ok(Some(SlashCommand::Recovery(RecoveryCommand::Off)))
    );
    assert_eq!(
        parse_slash_command("/recovery list"),
        Ok(Some(SlashCommand::Recovery(RecoveryCommand::List)))
    );
    assert_eq!(
        parse_slash_command("/recovery diff ck-abcdef"),
        Ok(Some(SlashCommand::Recovery(RecoveryCommand::Diff(
            "ck-abcdef".into()
        ))))
    );
    assert_eq!(
        parse_slash_command("/recovery restore ck-abcdef1234"),
        Ok(Some(SlashCommand::Recovery(RecoveryCommand::Restore(
            "ck-abcdef1234".into()
        ))))
    );
}

/// There is no free text on `/recovery`: trailing words on the
/// argument-less forms, a missing or extra id on diff/restore, and any
/// unknown sub-word are usage errors naming the remedy.
#[test]
fn recovery_malformed_forms_are_usage_errors() {
    for sub in ["status", "on", "off", "list"] {
        let error = parse_slash_command(&format!("/recovery {sub} extra")).unwrap_err();
        assert!(
            error
                .0
                .contains(&format!("recovery {sub} takes no argument")),
            "{error}"
        );
    }
    let diff_empty = parse_slash_command("/recovery diff").unwrap_err();
    assert!(
        diff_empty
            .0
            .contains("recovery diff requires an id: /recovery diff <id>"),
        "{diff_empty}"
    );
    let restore_empty = parse_slash_command("/recovery restore").unwrap_err();
    assert!(
        restore_empty
            .0
            .contains("recovery restore requires an id: /recovery restore <id>"),
        "{restore_empty}"
    );
    let restore_extra = parse_slash_command("/recovery restore a b").unwrap_err();
    assert!(
        restore_extra
            .0
            .contains("recovery restore takes exactly one id"),
        "{restore_extra}"
    );
    let diff_extra = parse_slash_command("/recovery diff a b").unwrap_err();
    assert!(
        diff_extra.0.contains("recovery diff takes exactly one id"),
        "{diff_extra}"
    );
    let unknown = parse_slash_command("/recovery nope").unwrap_err();
    assert!(
        unknown.0.contains(
            "unknown /recovery subcommand: nope (usage: /recovery, /recovery on, \
                       /recovery off, /recovery list, /recovery diff <id>, /recovery restore <id>)"
        ),
        "{unknown}"
    );
}
