#[test]
fn show_parses_the_single_id() {
    assert_eq!(
        parsed("investigation", "show abc-123").unwrap(),
        InvestigationCommand::Show {
            id: "abc-123".into()
        }
    );
    let empty = parsed("investigation", "show").unwrap_err();
    assert!(
        empty.0.contains("usage"),
        "a missing id is a usage error: {empty}"
    );
    // Two tokens is not an id; ids never contain whitespace.
    assert!(parsed("investigation", "show abc 123").is_err());
}

#[test]
fn delete_parses_the_id_and_optional_revision() {
    assert_eq!(
        parsed("investigation", "delete abc-123").unwrap(),
        InvestigationCommand::Delete {
            id: "abc-123".into(),
            revision: None
        }
    );
    assert_eq!(
        parsed("investigation", "delete abc-123 --revision 3").unwrap(),
        InvestigationCommand::Delete {
            id: "abc-123".into(),
            revision: Some(3)
        }
    );
    let not_a_number = parsed("investigation", "delete abc --revision x").unwrap_err();
    assert!(
        not_a_number.0.contains("--revision needs a number"),
        "a non-numeric --revision must be a usage error: {not_a_number}"
    );
    assert!(parsed("investigation", "delete abc extra").is_err());
}
