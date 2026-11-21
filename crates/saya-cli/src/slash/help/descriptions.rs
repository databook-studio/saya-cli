/// The bypass-composition sentence shared by the `/mode` help entry and both
/// `/mode` answers: under `bypass`, Plan still denies writes. One constant so
/// the surfaces state it in the same words; tests assert each surface contains
/// it verbatim. Kept beside the command it documents.
pub(crate) const PLAN_BYPASS_SENTENCE: &str = "Plan still denies writes under `bypass`";

/// The grouped, described listing printed by `/help` with no argument — a view
/// of [`crate::slash::catalog::CATALOGUE`], not a list of its own. Each of the
/// catalogue's categories prints its heading, then its rows in listing order,
/// each pairing the usage form with the shared one-line description. One
/// string so the TUI and headless paths render it identically; the transcript
/// word-wraps it at the terminal width.
pub(crate) fn help_text() -> String {
    let mut out = String::from("Slash commands:");
    for category in Category::ALL {
        out.push_str("\n\n");
        out.push_str(category.heading());
        for (usage, description) in crate::slash::catalog::listed_in(category) {
            out.push_str("\n  ");
            out.push_str(usage);
            out.push_str(" — ");
            out.push_str(description);
        }
    }
    out
}
