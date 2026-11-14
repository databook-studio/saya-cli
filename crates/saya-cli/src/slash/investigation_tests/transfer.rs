#[test]
fn export_parses_the_id_optional_overwrite_and_path() {
    assert_eq!(
        parsed("investigation", "export abc-123 out/results.json").unwrap(),
        InvestigationCommand::Export {
            id: "abc-123".into(),
            path: PathBuf::from("out/results.json"),
            overwrite: false
        }
    );
    // The path is the rest of the tail verbatim — spaces included.
    assert_eq!(
        parsed("investigation", "export abc --overwrite my file.json").unwrap(),
        InvestigationCommand::Export {
            id: "abc".into(),
            path: PathBuf::from("my file.json"),
            overwrite: true
        }
    );
    assert!(parsed("investigation", "export abc").is_err());
    let unknown = parsed("investigation", "export abc --bogus out.json").unwrap_err();
    assert!(
        unknown.0.contains("unknown investigation flag: --bogus"),
        "an unknown flag must be a usage error: {unknown}"
    );
}

#[test]
fn import_parses_the_path() {
    assert_eq!(
        parsed("investigation", "import defs/inv.json").unwrap(),
        InvestigationCommand::Import {
            path: PathBuf::from("defs/inv.json")
        }
    );
    assert_eq!(
        parsed("investigation", "import my defs.json").unwrap(),
        InvestigationCommand::Import {
            path: PathBuf::from("my defs.json")
        }
    );
    assert!(parsed("investigation", "import").is_err());
    let unknown = parsed("investigation", "import --bogus x").unwrap_err();
    assert!(
        unknown.0.contains("unknown investigation flag: --bogus"),
        "an unknown flag must be a usage error: {unknown}"
    );
}
