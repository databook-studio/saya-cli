/// The `/run` family stays registered, described and documented where a
/// reader decides to use it — and K8 hides it from the listing. A command
/// that parses but is absent from help is exactly the defect this test exists
/// to catch — a half-existing command; the hide is the one deliberate
/// exception, and `/help run` still carries the words.
#[test]
fn run_and_runs_stay_registered_and_described_but_unlisted() {
    for name in ["run", "runs"] {
        assert!(
            crate::slash::catalog::is_command(name),
            "{name} is registered"
        );
        // The description is the catalogue's even though the popup and the
        // listing no longer offer the row (K8), so read the rows themselves.
        assert!(
            crate::slash::catalog::all()
                .find(|spec| spec.name == name)
                .is_some_and(|spec| !spec.description.is_empty()),
            "{name} carries a one-line description"
        );
        let help = command_help(name).expect("per-command help exists");
        assert!(!help.is_empty(), "{name} help is not empty");
    }
    let listing = help_text();
    assert!(
        !listing.contains("/run <goal…> --allow <scopes>"),
        "the listing hides the /run usage (K8): {listing}"
    );
    assert!(
        !listing.contains("/runs [id]"),
        "the listing hides the /runs usage (K8): {listing}"
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

/// `/contracts` is one command over the knowledge store: eight verbs, one help
/// entry, and each verb named in the prose a reader decides with. The five old
/// spellings keep help of their own — it says they are deprecated aliases and
/// names the verb that replaces them — but the `/help` listing lists only the
/// canonical row.
#[test]
fn contracts_help_lists_the_eight_verbs_and_folds_the_old_names() {
    let summary = help_text();
    assert!(
        summary.contains("/contracts <list|show|queue|confirm|reject|confirm-all|export|import>"),
        "summary must show the /contracts verb row, got: {summary}"
    );
    for old in ["/queue", "/confirm", "/reject", "/approve-all"] {
        assert!(
            !summary.contains(old),
            "{old} is an alias; the listing shows canonical rows only, got: {summary}"
        );
    }

    let contracts = command_help("contracts").expect("contracts has help");
    // Every verb the row names is explained, so the row is not the only place a
    // reader learns what a verb does.
    for verb in [
        "list",
        "show",
        "queue",
        "confirm",
        "reject",
        "confirm-all",
        "export",
        "import",
    ] {
        assert!(
            contracts.contains(verb),
            "/contracts help must name the {verb} verb, got: {contracts}"
        );
    }
    // The show form marks its argument, and an example shows it.
    assert!(
        contracts.contains("<catalog.schema.object>"),
        "/contracts help must mark the show argument, got: {contracts}"
    );
    assert!(
        contracts.contains("analytics.public.orders"),
        "/contracts help must keep its example, got: {contracts}"
    );

    // The deprecated spellings keep help, and it says which verb replaces them.
    for (old, new) in [
        ("contract", "/contracts"),
        ("queue", "/contracts queue"),
        ("confirm", "/contracts confirm"),
        ("reject", "/contracts reject"),
        ("approve-all", "/contracts confirm-all"),
    ] {
        let help = command_help(old).unwrap_or_else(|| panic!("/{old} still has help"));
        assert!(
            help.contains("deprecated") && help.contains(new),
            "/{old} help must say it is deprecated and name {new}, got: {help}"
        );
    }
    // `/approve-all` reads like a job approval: its help says it never was one.
    let approve_all = command_help("approve-all").expect("approve-all still has help");
    assert!(
        approve_all.contains("knowledge claims"),
        "/approve-all help must say what it approves, got: {approve_all}"
    );
}

/// The consent mode has one grammar (the six-value mode parse) and every help
/// surface must name the whole vocabulary it parses — including `bypass`,
/// `read-only` and `never`. A mode that parses but no help names is the exact
/// drift the run grammar's equivalence tests (`scopes.rs`) catch, on the
/// session's own mode word. The clap docs, the `/approvals` alias's per-command
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
        listing.contains("/mode [ask|accept-edits|plan|bypass|read-only|never]"),
        "the /help listing must show the full mode vocabulary: {listing}"
    );
    assert!(
        !listing.contains("/approvals"),
        "/approvals is an alias: the listing shows canonical names only: {listing}"
    );
}

/// `/rotate` is `/compact --new-session`'s one-release alias, wired at every
/// touchpoint: the catalogue resolves it, it keeps its own popup description
/// (aliases are catalogue rows' aliases, each with its own description), the
/// listing shows the canonical `/compact [--new-session]` row instead, and the
/// per-command help states what carries over, what does not, and the
/// /compact-first requirement.
#[test]
fn rotate_is_an_alias_listed_described_and_helped() {
    assert!(
        crate::slash::catalog::resolve("rotate") == Some("compact"),
        "rotate is /compact's alias"
    );
    assert_eq!(
        description_for("rotate"),
        Some("Deprecated alias for /compact --new-session (removed next release)"),
        "the alias keeps its own one-line description"
    );
    let listing = help_text();
    assert!(
        listing.contains("/compact [--new-session]"),
        "the listing shows the canonical row: {listing}"
    );
    let help = command_help("rotate").expect("rotate has per-command help");
    for stated in ["/compact --new-session", "grants", "resumable"] {
        assert!(
            help.contains(stated),
            "the /rotate help states {stated:?}: {help}"
        );
    }
}
