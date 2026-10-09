use super::*;

#[test]
fn every_subcommand_and_argument_has_help_text() {
    let root = Cli::command();
    let mut offenders: Vec<String> = Vec::new();
    walk(&root, &mut offenders);

    if !offenders.is_empty() {
        panic!(
            "blank --help entries (no description found):\n  - {}\n\
             Add a `///` doc comment (subcommands) or `///` on the arg field \
             (arguments) so `--help` is never silent.",
            offenders.join("\n  - ")
        );
    }
}

#[test]
fn config_doctor_help_describes_configuration_without_reachability_claims() {
    let mut cmd = Cli::command();
    let help = cmd
        .find_subcommand_mut("config")
        .expect("`config` subcommand exists")
        .find_subcommand_mut("doctor")
        .expect("`config doctor` subcommand exists")
        .render_help()
        .to_string();

    assert!(
        help.contains("configured"),
        "help must describe configuration: {help}"
    );
    assert!(
        !help.contains("reachable"),
        "help must not claim provider reachability: {help}"
    );
}
