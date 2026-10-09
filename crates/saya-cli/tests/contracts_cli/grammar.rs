/// `saya contracts review …` no longer parses: the subcommand is gone. clap
/// reports an unrecognized subcommand rather than
/// reaching the runtime `AmbiguousReview` path the old `review` had.
#[test]
fn review_subcommand_no_longer_parses() {
    use clap::Parser;
    // `--decision confirm` is `decide`'s vocabulary, not `review`'s; the point
    // is the *subcommand name* `review` is rejected regardless of its args.
    let parsed = Cli::try_parse_from([
        "saya",
        "contracts",
        "review",
        "ki-deadbeef",
        "--confirm",
        "--reject",
    ]);
    assert!(
        parsed.is_err(),
        "`contracts review` must not parse after retirement: {parsed:?}"
    );
    let err = parsed.err().unwrap().to_string();
    assert!(
        !err.contains("choose exactly one"),
        "the runtime AmbiguousReview message must not be reachable: {err}"
    );
}

/// `saya contracts decide …` parses and the illegal input `review` caught at
/// runtime (`--confirm --reject` together) is now unexpressible: `--decision`
/// is a single ValueEnum, so two decisions or an unknown one is a parse error.
#[test]
fn decide_subcommand_parses_and_decision_is_a_single_value_enum() {
    use clap::Parser;
    let parsed = Cli::try_parse_from([
        "saya",
        "contracts",
        "decide",
        "ki-deadbeef",
        "--decision",
        "confirm",
    ]);
    let cli = parsed.expect("`contracts decide` parses");
    let command = cli.command.expect("a subcommand was given");
    let Command::Contracts {
        command: ContractsCommand::Decide { decision, .. },
    } = command
    else {
        panic!("parsed to Decide, got {command:?}");
    };
    assert_eq!(decision, ReviewDecisionArg::Confirm);

    // Two decisions cannot be expressed: clap rejects a repeated `--decision`
    // with a different value.
    let two = Cli::try_parse_from([
        "saya",
        "contracts",
        "decide",
        "ki-deadbeef",
        "--decision",
        "confirm",
        "--decision",
        "reject",
    ]);
    assert!(
        two.is_err(),
        "two --decision values must not parse: {two:?}"
    );

    // An unknown decision value is a parse error, not a runtime path.
    let unknown = Cli::try_parse_from([
        "saya",
        "contracts",
        "decide",
        "ki-deadbeef",
        "--decision",
        "maybe",
    ]);
    assert!(
        unknown.is_err(),
        "an unknown --decision must not parse: {unknown:?}"
    );
}

/// `saya contracts --help` advertises `decide` and not `review`: the retired
/// command left the help surface, and its replacement is what a user finds.
#[test]
fn contracts_help_advertises_decide_not_review() {
    use clap::CommandFactory;
    // Render the help for the `contracts` subcommand. `find_subcommand`
    // returns a `&Command` but `render_help` takes `&mut self`, so clone the
    // subcommand. `render_help` returns a `StyledStr` (clap 4.6) that
    // stringifies to the plain help text.
    let help = Cli::command()
        .find_subcommand("contracts")
        .expect("contracts subcommand exists")
        .clone()
        .render_help()
        .to_string();
    assert!(
        help.contains("decide"),
        "contracts --help must advertise decide: {help}"
    );
    // `review` appears in other subcommand prose (e.g. `queue`'s "pending
    // review"), so assert on the subcommand *entry* line, not a bare substring.
    assert!(
        !help
            .lines()
            .any(|line| line.trim_start().starts_with("review ")),
        "contracts --help must not list a `review` subcommand: {help}"
    );
}
