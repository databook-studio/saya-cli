//! Unit tests for `cli.rs` types: the Debug impls the adapters embed in
//! diagnostics must not carry user-supplied values (F-5).

use crate::cli::InvestigationCommand;

/// The run command's Debug names its `--param` bindings and never prints a
/// value (F-5): the TUI replay task and the session command enum embed the
/// command in their own Debug, so a bound value must not ride diagnostics
/// with it. A malformed entry with no `=` is fully redacted, and the other
/// fields still print as the derive would.
#[test]
fn the_run_command_debug_names_params_without_values() {
    let command = InvestigationCommand::Run {
        id: "inv-run-1".to_owned(),
        connection: Some("local".to_owned()),
        revalidate: false,
        report: None,
        rows: None,
        overwrite: false,
        params: vec![
            "token=sk-live-abc123".to_owned(),
            "label=a=b".to_owned(),
            "malformed".to_owned(),
        ],
    };
    let rendered = format!("{command:?}");
    assert!(
        rendered.contains("token=…") && rendered.contains("label=…"),
        "each bound name prints with a value marker: {rendered}"
    );
    assert!(
        !rendered.contains("sk-live-abc123") && !rendered.contains("a=b"),
        "no value or value fragment prints: {rendered}"
    );
    assert!(
        !rendered.contains("malformed"),
        "an entry without a name is fully redacted: {rendered}"
    );
    assert!(
        rendered.contains("local"),
        "the other fields still print: {rendered}"
    );
}
