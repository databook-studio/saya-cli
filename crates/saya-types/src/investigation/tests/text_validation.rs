use super::*;

#[test]
fn name_bounds_are_unicode_scalars_not_bytes() {
    let mut definition = valid_definition();
    definition.name = "é".repeat(MAX_NAME_CHARS);
    definition
        .validate()
        .expect("80 unicode scalars fit even in 160 bytes");
    definition.name = "é".repeat(MAX_NAME_CHARS + 1);
    assert_eq!(definition.validate(), Err(InvestigationError::InvalidName));
}

#[test]
fn name_rejects_trimmed_empty_and_control_characters() {
    let mut definition = valid_definition();
    definition.name = "   ".to_string();
    assert_eq!(definition.validate(), Err(InvestigationError::InvalidName));
    definition.name = "a\nb".to_string();
    assert_eq!(
        definition.validate(),
        Err(InvestigationError::ControlCharacter)
    );
    definition.name = "  Revenue  ".to_string();
    definition
        .validate()
        .expect("surrounding whitespace is tolerated; trimming is the caller's concern");
}

#[test]
fn description_bounds_allow_only_newline_control_characters() {
    let mut definition = valid_definition();
    definition.description = Some("x".repeat(MAX_DESCRIPTION_BYTES));
    definition.validate().expect("2048 bytes fit");
    definition.description = Some("x".repeat(MAX_DESCRIPTION_BYTES + 1));
    assert_eq!(
        definition.validate(),
        Err(InvestigationError::DescriptionTooLong)
    );
    definition.description = Some("line one\nline two".to_string());
    definition
        .validate()
        .expect("newlines are allowed in a description");
    definition.description = Some("tab\tstop".to_string());
    assert_eq!(
        definition.validate(),
        Err(InvestigationError::ControlCharacter)
    );
}

#[test]
fn sql_bounds_and_trimmed_emptiness() {
    let mut definition = valid_definition();
    definition.sql = "s".repeat(MAX_SQL_BYTES);
    definition.validate().expect("64 KiB of sql fits");
    definition.sql = "s".repeat(MAX_SQL_BYTES + 1);
    assert_eq!(definition.validate(), Err(InvestigationError::InvalidSql));
    definition.sql = " \n ".to_string();
    assert_eq!(definition.validate(), Err(InvestigationError::InvalidSql));
}

#[test]
fn sql_rejects_terminal_control_characters() {
    let mut definition = valid_definition();
    for sql in [
        // ESC opening an OSC sequence inside a comment.
        "-- \u{1b}]0;pwned\u{7}\nselect 1",
        // ESC opening a CSI sequence.
        "-- \u{1b}[31mred\nselect 1",
        // BEL on its own.
        "select 1 -- \u{7}",
        // NUL.
        "select\u{0}1",
        // U+009B, the single-character C1 CSI.
        "select 1 -- \u{9b}31m",
        // DEL.
        "select 1 -- \u{7f}",
    ] {
        definition.sql = sql.to_string();
        assert_eq!(
            definition.validate(),
            Err(InvestigationError::ControlCharacter),
            "{sql:?} must be refused"
        );
    }
}

#[test]
fn sql_keeps_newlines_tabs_and_crlf() {
    let mut definition = valid_definition();
    definition.sql = "select 1,\r\n\t2\r\nfrom t\n".to_string();
    definition
        .validate()
        .expect("newlines, tabs, and CRLF are still allowed");
}

#[test]
fn connection_bounds_and_charset() {
    let mut definition = valid_definition();
    definition.connection = "a".repeat(MAX_CONNECTION_CHARS);
    definition
        .validate()
        .expect("64 characters of connection alias fit");
    definition.connection = "a".repeat(MAX_CONNECTION_CHARS + 1);
    assert_eq!(
        definition.validate(),
        Err(InvestigationError::InvalidConnection)
    );
    definition.connection = String::new();
    assert_eq!(
        definition.validate(),
        Err(InvestigationError::InvalidConnection)
    );
    definition.connection = "prod/db".to_string();
    assert_eq!(
        definition.validate(),
        Err(InvestigationError::InvalidConnection)
    );
    definition.connection = "prod.db_1-x".to_string();
    definition
        .validate()
        .expect("dots, underscores, and dashes are allowed");
}
