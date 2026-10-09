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
