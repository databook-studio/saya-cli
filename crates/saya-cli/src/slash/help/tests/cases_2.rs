/// `/columns` is a slash command, so it needs a `/help` entry, a listing
/// line, and a description shared with the popup — and the help must say
/// that copy still yields the full table (the view filter is paint-only).
/// The path that does is Ctrl+B (`copy_transcript` keeps the full
/// untruncated table text); Ctrl+Y copies the last assistant block only.
#[test]
fn columns_has_help_listing_and_copy_guarantee() {
    assert!(
        registry::KNOWN_COMMANDS.contains(&"columns"),
        "columns is registered"
    );
    let listing = help_text();
    assert!(
        listing.contains("/columns [name,name,… | all]"),
        "listing shows the /columns usage: {listing}"
    );
    let help = command_help("columns").expect("columns has help");
    assert!(
        help.contains("which columns"),
        "/columns help names what it selects: {help}"
    );
    assert!(
        help.contains("Ctrl+B"),
        "/columns help must say copy still yields the full table: {help}"
    );
    assert!(
        description_for("columns").is_some(),
        "columns has a popup description"
    );
}

/// `/report` is wired at every hand-maintained touchpoint the
/// cli-application standard names: the registry, the description table
/// (the popup's single source), the `/help` listing, and the per-command
/// help. The entry states the report's guarantees: built from the
/// captured result only, SQL + provenance by default, rows on request,
/// never queries, never browses or uploads, and data files belong to
/// `/export`.
#[test]
fn report_is_registered_listed_and_described() {
    assert!(
        registry::KNOWN_COMMANDS.contains(&"report"),
        "report is registered"
    );
    assert!(
        description_for("report").is_some(),
        "report has a popup description"
    );
    let listing = help_text();
    assert!(
        listing.contains("/report [--rows N] [--overwrite] <path>"),
        "the listing shows the /report usage: {listing}"
    );
    let help = command_help("report").expect("report has per-command help");
    for stated in ["Markdown", "provenance", "--rows", "--overwrite", "/export"] {
        assert!(
            help.contains(stated),
            "the /report help states {stated:?}: {help}"
        );
    }
    assert!(
        help.contains("never"),
        "the /report help states what it never does: {help}"
    );
}

// --- Fieldnotes phase 6, packet 3: output actions name their scope. -------
//
// Red tests first: the `/columns` long help claims Ctrl+Y copies the full
// table (it copies the last assistant block only), and the `/export` and
// `/chart` helps never say the query is run again from a fresh read.

/// The `/columns` filter is paint-only, and the help must not credit
/// Ctrl+Y with copying tables: `copy_last_answer` finds only
/// `BlockKind::Assistant` (`application/input_actions/clipboard.rs`).
#[test]
fn the_columns_help_does_not_claim_ctrl_y_copies_tables() {
    let help = command_help("columns").expect("columns has help");
    assert!(
        !help.contains("Ctrl+Y/Ctrl+B"),
        "must not claim both copy keys yield the full table: {help}"
    );
    assert!(
        !help.contains("copied by Ctrl+Y"),
        "must not claim Ctrl+Y copies the table: {help}"
    );
}

/// Removing the false Ctrl+Y half must not drop the true Ctrl+B half:
/// `copy_transcript` keeps the full untruncated table text outside the
/// visible window (`ui_snapshot_tables.rs`), so the help still names it.
#[test]
fn the_columns_help_still_names_the_path_that_does() {
    let help = command_help("columns").expect("columns has help");
    assert!(
        help.contains("Ctrl+B"),
        "must still tell the user Ctrl+B copies the full table: {help}"
    );
}

/// `/export` dispatches a fresh `SqlTask` from `lq.sql` (`dispatch/query.rs`),
/// so the help must say the query is run again and the file reflects that
/// fresh read, not the displayed table.
#[test]
fn export_help_says_the_query_is_run_again() {
    let help = command_help("export").expect("export has help");
    let lower = help.to_lowercase();
    assert!(
        lower.contains("run") && (lower.contains("again") || lower.contains("re-run")),
        "/export help must say the query is run again: {help}"
    );
    assert!(
        lower.contains("fresh"),
        "/export help must say the output reflects a fresh read, not the displayed table: {help}"
    );
}

/// `/chart` dispatches the same fresh `SqlTask` shape as `/export`, so its
/// help carries the same re-run statement.
#[test]
fn chart_help_says_the_query_is_run_again() {
    let help = command_help("chart").expect("chart has help");
    let lower = help.to_lowercase();
    assert!(
        lower.contains("run") && (lower.contains("again") || lower.contains("re-run")),
        "/chart help must say the query is run again: {help}"
    );
    assert!(
        lower.contains("fresh"),
        "/chart help must say the output reflects a fresh read, not the displayed table: {help}"
    );
}

/// No output help may predict a row count: the count is not known before
/// the re-run, so a "will copy/export N rows" string would be invented.
#[test]
fn no_output_help_promises_a_row_count() {
    for name in ["columns", "export", "chart"] {
        let help = command_help(name).expect("output command has help");
        let lower = help.to_lowercase();
        assert!(
            !lower.contains("will copy") && !lower.contains("will export"),
            "/{name} help must not promise a pre-action row count: {help}"
        );
        assert!(
            !lower.contains("n rows"),
            "/{name} help must not predict an N-rows count: {help}"
        );
    }
}

/// the listing groups commands under short headings, so 28 described
/// commands stay scannable and `/connect` lands beside `/include` under one
/// heading. Grouping is presentation only; it adds, renames, and removes
/// nothing.
#[test]
fn listing_groups_commands_under_headings() {
    let summary = help_text();
    // The five group headings the listing uses. Each is on its own line.
    for heading in [
        "Connections",
        "Provider, model & privacy",
        "Query & data",
        "Session",
        "Memory",
    ] {
        assert!(
            summary.lines().any(|line| line.trim() == heading),
            "listing must have a {heading:?} heading on its own line, got:\n{summary}"
        );
    }
}
