//! Parser tests for the `/investigation` slash adapter: every subcommand
//! translates into the same `InvestigationCommand` the headless `saya
//! investigation` parser produces, `/investigations` is `list`, and the
//! usage errors carry guidance without swallowing value text.

use super::parse_investigation_command;
use crate::cli::InvestigationCommand;
use crate::slash::parse_slash_command;
use std::path::PathBuf;

fn save(
    name: &str,
    description: Option<&str>,
    sql: Option<&str>,
    file: Option<&str>,
    connection: Option<&str>,
) -> InvestigationCommand {
    InvestigationCommand::Save {
        name: name.into(),
        description: description.map(Into::into),
        sql: sql.map(Into::into),
        file: file.map(PathBuf::from),
        connection: connection.map(Into::into),
    }
}

fn parsed(name: &str, tail: &str) -> Result<InvestigationCommand, crate::slash::SlashParseError> {
    parse_investigation_command(name, tail)
}

// --- save ------------------------------------------------------------------

/// The positional name is the slash shape (`/investigation save <name>`),
/// with no SQL — exactly the form that saves the latest successful query.
#[test]
fn save_parses_the_positional_name_with_no_sql() {
    assert_eq!(
        parsed("investigation", "save Total orders").unwrap(),
        save("Total orders", None, None, None, None)
    );
}

/// Explicit `--sql`/`--description`/`--connection` parse into the same
/// command the clap parser builds for the equivalent argv.
#[test]
fn save_parses_explicit_sql_description_and_connection() {
    assert_eq!(
        parsed(
            "investigation",
            "save recent --description daily counts --sql SELECT count(*) FROM orders --connection staging",
        )
        .unwrap(),
        save(
            "recent",
            Some("daily counts"),
            Some("SELECT count(*) FROM orders"),
            None,
            Some("staging"),
        )
    );
}

/// The slash save shape is positional (`/investigation save <name>`, the
/// objective's form); the clap `--name` spelling is refused with the usage
/// that shows the positional form, so a clap-doc reader recovers in one step.
#[test]
fn save_refuses_the_clap_name_flag_with_the_positional_usage() {
    let refused = parsed("investigation", "save --name foo").unwrap_err();
    assert!(
        refused.0.contains("unknown investigation flag: --name")
            && refused.0.contains("save <name>"),
        "--name must be refused with the positional usage: {refused}"
    );
}

/// A flag value runs to the next known flag or the tail's end, so SQL keeps
/// its interior whitespace verbatim — the saved SQL must be exact.
#[test]
fn save_sql_value_keeps_interior_whitespace() {
    assert_eq!(
        parsed("investigation", "save n --sql SELECT 'a  b' FROM t").unwrap(),
        save("n", None, Some("SELECT 'a  b' FROM t"), None, None)
    );
}

/// Missing name, an unknown flag before any value zone, and a valueless
/// value flag are usage errors.
#[test]
fn save_usage_errors() {
    let no_name = parsed("investigation", "save --sql SELECT 1").unwrap_err();
    assert!(
        no_name.0.contains("name"),
        "a save without a name must say so: {no_name}"
    );
    let unknown = parsed("investigation", "save --bogus x").unwrap_err();
    assert!(
        unknown.0.contains("unknown investigation flag: --bogus"),
        "an unknown flag must be a usage error: {unknown}"
    );
    let valueless = parsed("investigation", "save n --sql --connection x").unwrap_err();
    assert!(
        valueless.0.contains("--sql needs a value"),
        "a valueless --sql must be a usage error: {valueless}"
    );
    assert!(parsed("investigation", "save").is_err());
}

// --- list ------------------------------------------------------------------

#[test]
fn list_parses_bare_and_with_bounds() {
    assert_eq!(
        parsed("investigation", "list").unwrap(),
        InvestigationCommand::List {
            limit: None,
            offset: None
        }
    );
    assert_eq!(
        parsed("investigation", "list --limit 5 --offset 3").unwrap(),
        InvestigationCommand::List {
            limit: Some(5),
            offset: Some(3)
        }
    );
    let not_a_number = parsed("investigation", "list --limit abc").unwrap_err();
    assert!(
        not_a_number.0.contains("--limit needs a number"),
        "a non-numeric --limit must be a usage error: {not_a_number}"
    );
    // list takes no positional; a stray token is a usage error.
    assert!(parsed("investigation", "list all").is_err());
}

/// `/investigations` is `list`: bare and with the same bounds.
#[test]
fn investigations_alias_parses_as_list() {
    assert_eq!(
        parsed("investigations", "").unwrap(),
        InvestigationCommand::List {
            limit: None,
            offset: None
        }
    );
    assert_eq!(
        parsed("investigations", "--limit 7").unwrap(),
        InvestigationCommand::List {
            limit: Some(7),
            offset: None
        }
    );
    // And through the full slash parser, the alias lands on the same command.
    assert_eq!(
        parse_slash_command("/investigations").unwrap(),
        Some(crate::slash::SlashCommand::Investigation(
            InvestigationCommand::List {
                limit: None,
                offset: None
            }
        ))
    );
}

// --- show / delete ---------------------------------------------------------

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

// --- export / import -------------------------------------------------------

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

// --- run -------------------------------------------------------------------

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
            overwrite: false
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
            overwrite: false
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
            overwrite: false
        }
    );
    assert!(parsed("investigation", "run").is_err());
    assert!(parsed("investigation", "run abc extra").is_err());
}

// --- dispatch surface ------------------------------------------------------

/// Bare `/investigation` and an unknown subcommand are usage errors naming
/// the subcommands — never `None` (which would send the line to the agent).
#[test]
fn bare_investigation_and_unknown_subcommand_are_usage_errors() {
    let bare = parsed("investigation", "").unwrap_err();
    assert!(
        bare.0.contains("save") && bare.0.contains("run"),
        "the usage error names the subcommands: {bare}"
    );
    let unknown = parsed("investigation", "bogus").unwrap_err();
    assert!(
        unknown.0.contains("unknown investigation subcommand"),
        "an unknown subcommand is a usage error: {unknown}"
    );
    // Through the full parser too.
    assert!(parse_slash_command("/investigation").is_err());
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
        parse_slash_command("/investigation run abc --connection staging").unwrap(),
        Some(Inv(InvestigationCommand::Run {
            id: "abc".into(),
            connection: Some("staging".into()),
            revalidate: false,
            report: None,
            rows: None,
            overwrite: false
        }))
    );
}
