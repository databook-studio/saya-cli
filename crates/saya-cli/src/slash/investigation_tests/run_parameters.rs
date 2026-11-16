#[test]
fn run_parses_the_id_connection_and_revalidate() {
    assert_eq!(
        parsed("investigation", "run abc-123").unwrap(),
        InvestigationCommand::Run {
            id: "abc-123".into(),
            connection: None,
            revalidate: false,
            report: None,
            rows: None,
            overwrite: false,
            params: Vec::new()
        }
    );
    assert_eq!(
        parsed("investigation", "run abc --connection staging --revalidate").unwrap(),
        InvestigationCommand::Run {
            id: "abc".into(),
            connection: Some("staging".into()),
            revalidate: true,
            report: None,
            rows: None,
            overwrite: false,
            params: Vec::new()
        }
    );
    assert_eq!(
        parsed("investigation", "run abc --revalidate").unwrap(),
        InvestigationCommand::Run {
            id: "abc".into(),
            connection: None,
            revalidate: true,
            report: None,
            rows: None,
            overwrite: false,
            params: Vec::new()
        }
    );
    assert!(parsed("investigation", "run").is_err());
    assert!(parsed("investigation", "run abc extra").is_err());
}

/// `--param` is repeatable and each binding rides through verbatim — the
/// same `Run` command the clap parser builds.
#[test]
fn run_parses_repeatable_param_bindings() {
    assert_eq!(
        parsed(
            "investigation",
            "run abc --param region=east --param day=2024-01-02"
        )
        .unwrap(),
        InvestigationCommand::Run {
            id: "abc".into(),
            connection: None,
            revalidate: false,
            report: None,
            rows: None,
            overwrite: false,
            params: vec!["region=east".into(), "day=2024-01-02".into()]
        }
    );
    // A binding with no `=` rides through too; the operation parses it and
    // refuses, so the slash adapter does not pre-empt its message.
    assert_eq!(
        parsed("investigation", "run abc --param bogus").unwrap(),
        InvestigationCommand::Run {
            id: "abc".into(),
            connection: None,
            revalidate: false,
            report: None,
            rows: None,
            overwrite: false,
            params: vec!["bogus".into()]
        }
    );
    let valueless = parsed("investigation", "run abc --param").unwrap_err();
    assert!(
        valueless.0.contains("--param needs a value"),
        "a valueless --param is a usage error: {valueless}"
    );
}

/// `--param-spec` parses on save and edit as a repeatable flag.
#[test]
fn save_and_edit_parse_repeatable_param_specs() {
    assert_eq!(
        parsed(
            "investigation",
            "save By region --sql SELECT 1 WHERE r = :region --param-spec region:string:required"
        )
        .unwrap(),
        InvestigationCommand::Save {
            name: "By region".into(),
            description: None,
            sql: Some("SELECT 1 WHERE r = :region".into()),
            file: None,
            connection: None,
            param_specs: vec!["region:string:required".into()]
        }
    );
    assert_eq!(
        parsed(
            "investigation",
            "edit abc-123 --param-spec region:string --param-spec day:date"
        )
        .unwrap(),
        InvestigationCommand::Edit {
            id: "abc-123".into(),
            name: None,
            description: None,
            sql: None,
            file: None,
            param_specs: vec!["region:string".into(), "day:date".into()]
        }
    );
}
