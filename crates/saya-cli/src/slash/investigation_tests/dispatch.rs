/// Bare `/investigation` is the listing, and an unknown subcommand is a usage
/// error naming the subcommands — never `None` (which would send the line to
/// the agent).
#[test]
fn bare_investigation_lists_and_an_unknown_subcommand_is_a_usage_error() {
    assert_eq!(
        parsed("investigation", "").unwrap(),
        InvestigationCommand::List {
            limit: None,
            offset: None
        },
        "row 156: bare /investigation lists in the line REPL"
    );
    let unknown = parsed("investigation", "bogus").unwrap_err();
    assert!(
        unknown.0.contains("unknown investigation subcommand"),
        "an unknown subcommand is a usage error: {unknown}"
    );
    // Through the full parser too.
    assert_eq!(
        parse_slash_command("/investigation").unwrap(),
        Some(crate::slash::SlashCommand::Investigation(
            InvestigationCommand::List {
                limit: None,
                offset: None
            }
        ))
    );
    assert!(parse_slash_command("/investigation bogus").is_err());
}

/// Every subcommand parses through the full slash parser to the same
/// `InvestigationCommand`, so the adapter hands the shared operation the one
/// value the clap parser produces.
#[test]
fn every_subcommand_parses_through_the_slash_parser() {
    use crate::slash::SlashCommand::Investigation as Inv;
    assert_eq!(
        parse_slash_command("/investigation save Total orders").unwrap(),
        Some(Inv(save("Total orders", None, None, None, None)))
    );
    assert_eq!(
        parse_slash_command("/investigation list").unwrap(),
        Some(Inv(InvestigationCommand::List {
            limit: None,
            offset: None
        }))
    );
    assert_eq!(
        parse_slash_command("/investigation show abc-123").unwrap(),
        Some(Inv(InvestigationCommand::Show {
            id: "abc-123".into()
        }))
    );
    assert_eq!(
        parse_slash_command("/investigation delete abc-123").unwrap(),
        Some(Inv(InvestigationCommand::Delete {
            id: "abc-123".into(),
            revision: None
        }))
    );
    assert_eq!(
        parse_slash_command("/investigation export abc out.json").unwrap(),
        Some(Inv(InvestigationCommand::Export {
            id: "abc".into(),
            path: PathBuf::from("out.json"),
            overwrite: false
        }))
    );
    assert_eq!(
        parse_slash_command("/investigation import out.json").unwrap(),
        Some(Inv(InvestigationCommand::Import {
            path: PathBuf::from("out.json")
        }))
    );
    assert_eq!(
        parse_slash_command("/investigation edit abc-123 --name Renamed").unwrap(),
        Some(Inv(InvestigationCommand::Edit {
            id: "abc-123".into(),
            name: Some("Renamed".into()),
            description: None,
            sql: None,
            file: None,
            param_specs: Vec::new()
        }))
    );
    assert_eq!(
        parse_slash_command("/investigation run abc --connection staging").unwrap(),
        Some(Inv(InvestigationCommand::Run {
            id: "abc".into(),
            connection: Some("staging".into()),
            revalidate: false,
            report: None,
            rows: None,
            overwrite: false,
            params: Vec::new()
        }))
    );
}
