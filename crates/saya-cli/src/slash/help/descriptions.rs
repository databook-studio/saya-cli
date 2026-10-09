/// Looks up the one-line description for a command name, or `None` if unknown.
pub(crate) fn description_for(name: &str) -> Option<&'static str> {
    COMMAND_DESCRIPTIONS
        .iter()
        .find(|(candidate, _)| *candidate == name)
        .map(|(_, description)| *description)
}

/// The bypass-composition sentence shared by the `/mode` help entry and both
/// `/mode` answers: under `bypass`, Plan still denies writes. One constant so
/// the surfaces state it in the same words; tests assert each surface contains
/// it verbatim.
/// The grouped, described listing printed by `/help` with no argument. Built
/// from [`LISTING_GROUPS`] (the usage form and group heading) plus the
/// description text from [`COMMAND_DESCRIPTIONS`], so every line carries a
/// description and `/connect` lands beside `/include` under one heading. One
/// string so the TUI and headless paths render it identically; the transcript
/// word-wraps it at the terminal width.
pub(crate) fn help_text() -> String {
    let mut out = String::from("Slash commands:");
    for (heading, commands) in LISTING_GROUPS {
        out.push_str("\n\n");
        out.push_str(heading);
        for (name, usage) in *commands {
            // Every shown command must have a description in the single source;
            // a missing one is a bug, not a bare-syntax line.
            out.push_str("\n  ");
            out.push_str(usage);
            // A missing description is a bug the listing test catches. Degrade
            // to bare syntax rather than panicking: `/help` runs inside a live
            // TUI session, where a panic costs the session and the terminal
            // state, and a line without its description still works.
            if let Some(description) = description_for(name) {
                out.push_str(" — ");
                out.push_str(description);
            }
        }
    }
    out
}
