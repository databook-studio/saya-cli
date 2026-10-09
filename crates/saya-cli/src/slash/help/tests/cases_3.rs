/// The `/run` family is wired at every hand-maintained touchpoint, not just
/// the parser: the registry, the description table (the popup's single
/// source), the `/help` listing, and the per-command help. A command that
/// parses but is absent from help is exactly the defect this test exists to
/// catch — a half-existing command.
#[test]
fn run_and_runs_are_registered_listed_and_described() {
    for name in ["run", "runs"] {
        assert!(
            registry::KNOWN_COMMANDS.contains(&name),
            "{name} is registered"
        );
        assert!(
            description_for(name).is_some(),
            "{name} has a popup description"
        );
        let help = command_help(name).expect("per-command help exists");
        assert!(!help.is_empty(), "{name} help is not empty");
    }
    let listing = help_text();
    assert!(
        listing.contains("/run <goal…> --allow <scopes>"),
        "the listing shows the /run usage: {listing}"
    );
    assert!(
        listing.contains("/runs [id]"),
        "the listing shows the /runs usage: {listing}"
    );
    // `/run cancel` is reachable and documented where a reader decides how to
    // stop a run.
    let run_help = command_help("run").expect("run has help");
    assert!(
        run_help.contains("/run cancel"),
        "the /run help names the cancel form: {run_help}"
    );
    assert!(
        run_help.contains("paused 6"),
        "the /run help names the paused exit the headless scheme documents: {run_help}"
    );
}

/// the merged `/contracts` command has one help entry covering both
/// forms, and the optional argument that selects the operation is obvious —
/// the `[…]` bracket, the prose ("or … when you name it"), and both examples.
/// `/contract` stays documented so `/help contract` does not say "no help".
#[test]
fn contracts_help_covers_both_forms_and_marks_the_optional_argument() {
    let summary = help_text();
    // The one-line summary lists the merged spelling, with the optional table
    // in brackets — not two separate `/contracts` and `/contract <table>` lines.
    assert!(
        summary.contains("/contracts [table]"),
        "summary must show the merged /contracts [table] form, got: {summary}"
    );
    assert!(
        !summary.contains("/contract <table>"),
        "summary must not keep the old separate /contract <table> entry, got: {summary}"
    );

    let contracts = command_help("contracts").expect("contracts has help");
    // Both operations are named: listing every contract, and showing one.
    assert!(
        contracts.contains("list every recalled contract"),
        "/contracts help must name the list form, got: {contracts}"
    );
    assert!(
        contracts.contains("show one object's contract"),
        "/contracts help must name the show form, got: {contracts}"
    );
    // The optional argument is marked: the `[…]` bracket signals "absent or
    // present", which is what selects list-vs-show.
    assert!(
        contracts.contains("[catalog.schema.object]"),
        "/contracts help must mark the optional argument with brackets, got: {contracts}"
    );
    // Both forms get an example so a reader sees the argument selecting the
    // operation, not two separate commands.
    assert!(
        contracts.contains("/contracts") && contracts.contains("analytics.public.orders"),
        "/contracts help must show a no-arg and a with-arg example, got: {contracts}"
    );

    // `/contract` is an alias, so its help exists and describes the same
    // merged behaviour — it must not be the old Show-only usage.
    let contract = command_help("contract").expect("contract still has help");
    assert!(
        contract.contains("alias"),
        "/contract help must name itself an alias of /contracts, got: {contract}"
    );
    assert!(
        contract.contains("show one object's contract"),
        "/contract help must describe the merged show form, got: {contract}"
    );
}

/// The approvals grammar has one authority (`ApprovalPolicy::from_str`)
/// and every help surface must name the whole vocabulary it parses —
/// including `bypass`. A mode that parses but no help names is the exact
/// drift the run grammar's parity test (`scopes.rs`) exists to catch, on
/// the session's own mode word. The clap doc, the `/approvals` per-command
/// help, and the `/help` listing must all carry it.
#[test]
fn every_help_surface_names_bypass() {
    use clap::CommandFactory as _;
    let cmd = crate::cli::Cli::command();
    let approval_mode = cmd
        .get_arguments()
        .find(|arg| arg.get_id() == "approval_mode")
        .expect("`--approval-mode` is declared on the global options");
    let clap_doc = approval_mode
        .get_long_help()
        .or_else(|| approval_mode.get_help())
        .expect("the --approval-mode doc comment reaches clap")
        .to_string();
    assert!(
        clap_doc.contains("bypass"),
        "the clap `--approval-mode` doc must name the `bypass` value, got: {clap_doc}"
    );

    let help = command_help("approvals").expect("approvals has per-command help");
    assert!(
        help.contains("bypass"),
        "the /approvals help must name the `bypass` value, got: {help}"
    );

    let listing = help_text();
    assert!(
        listing.contains("/approvals [ask|read-only|never|bypass]"),
        "the /help listing must show the full mode vocabulary: {listing}"
    );
}

/// `/rotate` is wired at every hand-maintained touchpoint the
/// cli-application standard names: the registry, the description table
/// (the popup's single source), the `/help` listing (the Session group),
/// and the per-command help that states what carries over, what does
/// not, and the /compact-first requirement.
#[test]
fn rotate_is_registered_listed_and_described() {
    assert!(
        registry::KNOWN_COMMANDS.contains(&"rotate"),
        "rotate is registered"
    );
    assert_eq!(
        description_for("rotate"),
        Some(
            "Continue in a new, smaller session that keeps your summary and notes (run /compact first)"
        ),
        "the description is the shared one-liner"
    );
    let listing = help_text();
    assert!(
        listing.contains("/rotate"),
        "the listing shows the /rotate usage: {listing}"
    );
    let help = command_help("rotate").expect("rotate has per-command help");
    for stated in ["/compact", "grants", "resumable"] {
        assert!(
            help.contains(stated),
            "the /rotate help states {stated:?}: {help}"
        );
    }
}
