/// `/investigation edit <id> …` carries the positional id and the same
/// `--name`/`--description`/`--sql`/`--file`/`--param-spec` flags the clap
/// parser takes.
#[test]
fn edit_parses_the_id_and_the_edit_flags() {
    assert_eq!(
        parsed("investigation", "edit abc-123").unwrap(),
        InvestigationCommand::Edit {
            id: "abc-123".into(),
            name: None,
            description: None,
            sql: None,
            file: None,
            param_specs: Vec::new()
        }
    );
    assert_eq!(
        parsed(
            "investigation",
            "edit abc-123 --name Renamed --description new words --sql SELECT 1 AS one"
        )
        .unwrap(),
        InvestigationCommand::Edit {
            id: "abc-123".into(),
            name: Some("Renamed".into()),
            description: Some("new words".into()),
            sql: Some("SELECT 1 AS one".into()),
            file: None,
            param_specs: Vec::new()
        }
    );
    assert_eq!(
        parsed("investigation", "edit abc-123 --file replacement.sql").unwrap(),
        InvestigationCommand::Edit {
            id: "abc-123".into(),
            name: None,
            description: None,
            sql: None,
            file: Some(PathBuf::from("replacement.sql")),
            param_specs: Vec::new()
        }
    );
    // SQL keeps its interior whitespace verbatim.
    assert_eq!(
        parsed("investigation", "edit abc-123 --sql SELECT 'a  b' FROM t").unwrap(),
        InvestigationCommand::Edit {
            id: "abc-123".into(),
            name: None,
            description: None,
            sql: Some("SELECT 'a  b' FROM t".into()),
            file: None,
            param_specs: Vec::new()
        }
    );
}

/// A missing id, an unknown flag, a valueless value flag, and a stray
/// positional are usage errors.
#[test]
fn edit_usage_errors() {
    let no_id = parsed("investigation", "edit --name Renamed").unwrap_err();
    assert!(
        no_id.0.contains("expected a single id"),
        "a missing id must be a usage error: {no_id}"
    );
    let bare = parsed("investigation", "edit").unwrap_err();
    assert!(
        bare.0.contains("usage"),
        "bare edit is a usage error: {bare}"
    );
    let unknown = parsed("investigation", "edit abc --bogus x").unwrap_err();
    assert!(
        unknown.0.contains("unknown investigation flag: --bogus"),
        "an unknown flag must be a usage error: {unknown}"
    );
    let valueless = parsed("investigation", "edit abc --sql --name x").unwrap_err();
    assert!(
        valueless.0.contains("--sql needs a value"),
        "a valueless --sql must be a usage error: {valueless}"
    );
    let extra = parsed("investigation", "edit abc extra --name Renamed").unwrap_err();
    assert!(
        extra.0.contains("expected a single id"),
        "a second positional must be a usage error: {extra}"
    );
}
