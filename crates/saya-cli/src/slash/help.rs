//! Help text for the slash commands: the grouped listing (`/help` with no
//! argument) and the per-command usage + example (`/help <command>`).
//!
//! Extracted from `slash.rs` to keep that file under the size cap. `slash.rs`
//! re-exports [`help_for`] so the existing `crate::slash::help_for` path used by
//! the session command layer keeps resolving.
//!
//! [`COMMAND_DESCRIPTIONS`] is the single source of the one-line description per
//! command. The completion popup in `interactive::tui::complete` reads it via
//! [`description_for`], and the `/help` listing here reads it too — so the popup
//! and the listing can never drift into two hand-maintained copies. The listing
//! only adds presentation the popup does not carry: a usage form, a group
//! heading, and which of the alias-folded commands to show.

/// The one-line description per command — **the single source**. Keyed by the
/// names in [`crate::slash::registry::KNOWN_COMMANDS`]; a test asserts the two
/// never drift apart. `interactive::tui::complete` reads this for its popup, and
/// the `/help` listing reads it here, so the two surfaces share one copy.
///
/// `/connect` and `/include` carry the contrast (one replaces, one adds a
/// secondary) so a user reading the listing can tell them apart without two more
/// `/help <name>` round-trips.
pub(crate) const COMMAND_DESCRIPTIONS: &[(&str, &str)] = &[
    ("connect", "Replace the active database profile"),
    ("connections", "List configured database connections"),
    ("include", "Add a secondary database profile to query scope"),
    ("exclude", "Remove a database profile from query scope"),
    ("provider", "Set or view the AI provider"),
    ("model", "Set or view the AI model"),
    ("privacy", "Enable or disable data sharing privacy"),
    ("approvals", "Set approval policy for tool execution"),
    ("schema", "Inspect or refresh database schema"),
    ("doctor", "Diagnose config: secrets, provider endpoint"),
    ("usage", "Show session token usage and cache hit rate"),
    ("thinking", "Toggle display of the model's chain-of-thought"),
    ("sql", "Run a raw SQL query against the active profile"),
    ("export", "Export the last query result as CSV or JSON"),
    ("chart", "Render the last query as an HTML chart"),
    ("explain", "Explain the given or last SQL statement"),
    ("clear", "Clear current session context"),
    ("history", "List saved sessions as text"),
    (
        "sessions",
        "Browse saved sessions; opens a picker in the TUI",
    ),
    ("resume", "Resume a saved session by id"),
    ("contracts", "List contracts, or show one object's contract"),
    ("contract", "Alias for /contracts"),
    ("remember", "Store a confirmed contract claim"),
    ("forget", "Tombstone a contract claim so recall excludes it"),
    ("queue", "Show pending candidate claims awaiting review"),
    ("confirm", "Confirm a pending candidate claim by id prefix"),
    ("reject", "Reject a pending candidate claim by id prefix"),
    (
        "approve-all",
        "Approve the whole review queue (needs --yes)",
    ),
    ("help", "Show help for slash commands"),
    ("exit", "Exit the REPL"),
    ("quit", "Exit the REPL"),
];

/// Looks up the one-line description for a command name, or `None` if unknown.
pub(crate) fn description_for(name: &str) -> Option<&'static str> {
    COMMAND_DESCRIPTIONS
        .iter()
        .find(|(candidate, _)| *candidate == name)
        .map(|(_, description)| *description)
}

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

/// Listing-only presentation: the group heading, command name, and usage form
/// for each shown command, in display order. The description text is NOT here
/// — it comes from [`COMMAND_DESCRIPTIONS`] via [`description_for`], so there is
/// one copy. The alias spellings `/contract` and `/quit` are folded under
/// `/contracts` and `/exit` (the listing must not show `/contract <table>`), so
/// this lists the canonical form of each command, not every alias.
const LISTING_GROUPS: &[(&str, &[(&str, &str)])] = &[
    (
        "Connections",
        &[
            ("connect", "/connect <profile>"),
            ("connections", "/connections"),
            ("include", "/include <profile>"),
            ("exclude", "/exclude <profile>"),
        ],
    ),
    (
        "Provider, model & privacy",
        &[
            ("provider", "/provider [name]"),
            ("model", "/model [name]"),
            ("privacy", "/privacy [on|off]"),
            ("approvals", "/approvals [ask|read-only|never]"),
        ],
    ),
    (
        "Query & data",
        &[
            ("schema", "/schema [refresh]"),
            ("sql", "/sql <query>"),
            ("export", "/export <path>"),
            ("chart", "/chart [type] [path]"),
            ("explain", "/explain [sql]"),
        ],
    ),
    (
        "Session",
        &[
            ("clear", "/clear"),
            ("history", "/history"),
            ("sessions", "/sessions"),
            ("resume", "/resume <id>"),
            ("doctor", "/doctor"),
            ("usage", "/usage"),
            ("thinking", "/thinking [on|off]"),
            ("help", "/help [command]"),
            ("exit", "/exit  (alias /quit)"),
        ],
    ),
    (
        "Memory",
        &[
            ("contracts", "/contracts [table]"),
            ("remember", "/remember <table> <kind> <value…>"),
            ("forget", "/forget <id>"),
            ("queue", "/queue [limit]"),
            ("confirm", "/confirm <prefix>"),
            ("reject", "/reject <prefix>"),
            ("approve-all", "/approve-all [--yes] [limit]"),
        ],
    ),
];

/// Returns a short usage and example string for a known slash command, or `None` if unknown.
pub(crate) fn command_help(name: &str) -> Option<&'static str> {
    let clean_name = name.trim_start_matches('/').to_lowercase();
    match clean_name.as_str() {
        "connect" => {
            Some("connect <profile> — set the active database profile. Example: /connect prod")
        }
        "connections" => Some(
            "connections — list configured database connection profiles. Example: /connections",
        ),
        "include" => Some(
            "include <profile> — include an additional database profile. Example: /include staging",
        ),
        "exclude" => {
            Some("exclude <profile> — exclude a database profile. Example: /exclude staging")
        }
        "provider" => {
            Some("provider [name] — view or set the AI provider. Example: /provider anthropic")
        }
        "model" => Some("model [name] — view or set the AI model. Example: /model gpt-4o"),
        "privacy" => {
            Some("privacy [on|off] — view or toggle cloud data sharing. Example: /privacy off")
        }
        "approvals" => Some(
            "approvals [ask|read-only|never] — view or set tool execution approval policy. Example: /approvals ask",
        ),
        "schema" => Some(
            "schema [refresh] — display or refresh database schema context. Example: /schema refresh",
        ),
        "sql" => Some(
            "sql <query> — execute a raw SQL query directly. Example: /sql SELECT * FROM users LIMIT 10;",
        ),
        "export" => Some(
            "export <path> — write the last query's rows to a .csv or .json file. Example: /export results.csv",
        ),
        "chart" => Some(
            "chart [type] [path] — render the last query as an interactive HTML chart and open it. type: bar|line|area|pie|doughnut|scatter (default auto)",
        ),
        "explain" => Some(
            "explain [sql] — show the query plan (EXPLAIN) for the given SQL, or the last query if omitted",
        ),
        "clear" => Some("clear — clear the conversation and context. Example: /clear"),
        "history" => Some("history — list saved sessions as text. Example: /history"),
        "sessions" => {
            Some("sessions — browse saved sessions; opens a picker in the TUI. Example: /sessions")
        }
        "doctor" => Some(
            "doctor — diagnose configuration: secrets resolve? provider endpoint? Example: /doctor",
        ),
        "usage" => Some(
            "usage — show session token usage: input, output, reasoning, cached input, cache creation, and the cache hit rate. The hit rate is Σcached / Σinput across all turns (a ratio of sums, not a mean of per-turn rates). Fields the provider did not report show —; the hit rate shows 'unknown' when no turn reported cached tokens (absent is not zero). Example: /usage",
        ),
        "thinking" => Some(
            "thinking [on|off] — toggle display of the model's chain-of-thought in the transcript. Off by default: thinking is verbose (often longer than the answer) and restates database contents in prose. With no argument, toggles; with on/off, sets explicitly. Display only — reasoning is never written to a saved session, and Ctrl+B (copy transcript) excludes it. Example: /thinking on",
        ),
        "resume" => Some("resume <id> — resume a previous session by ID. Example: /resume 12345"),
        "contracts" => Some(
            "contracts [catalog.schema.object] — list every recalled contract for the active profile, or show one object's contract when you name it. Example: /contracts   or   /contracts analytics.public.orders",
        ),
        // `/contract` is a silent alias of the merged `/contracts` command,
        // so `/help contract` returns the same help rather than "no help".
        "contract" => Some(
            "contract [catalog.schema.object] — alias for /contracts: list every recalled contract, or show one object's contract when you name it. Example: /contract analytics.public.orders",
        ),
        "remember" => Some(
            "remember <catalog.schema.object> <kind> <value…> [because <reason…>] — store a confirmed claim. Kinds: description, alias, grain, time-column, column-description <column> <value…>, column-role <column> <role>. The optional `because <reason…>` (directive kinds only) records why the claim holds. Example: /remember analytics.public.orders time-column created_at because orders complete on return",
        ),
        "forget" => Some(
            "forget <claim-id> — tombstone a claim so recall excludes it. Example: /forget abc-123",
        ),
        "queue" => {
            Some("queue [limit] — list candidate claims awaiting review. Example: /queue 20")
        }
        "confirm" => Some(
            "confirm <claim-id-prefix> — confirm the claim named by its short id prefix (the ki-xxxx form /contracts shows). Example: /confirm ki-a86a3f",
        ),
        "reject" => Some(
            "reject <claim-id-prefix> — reject the claim named by its short id prefix. Example: /reject ki-a86a3f",
        ),
        "approve-all" => Some(
            "approve-all [--yes] [limit] — approve every candidate in the review queue: the same set /queue shows. Each candidate still gets the per-item validation /confirm applies, so some may be refused; every approval and every refusal is reported by id. Without --yes the queue is printed and nothing is approved. Example: /approve-all --yes",
        ),
        "help" => Some(
            "help [command] — display general help or detailed usage for a command. Example: /help connect",
        ),
        "exit" | "quit" => Some("exit — exit the interactive CLI session. Example: /exit"),
        _ => None,
    }
}

/// Returns command-specific help for a topic, or general help text if `topic` is `None`.
pub(crate) fn help_for(topic: Option<&str>) -> String {
    match topic {
        Some(name) => {
            let clean = name.trim_start_matches('/');
            match command_help(clean) {
                Some(help) => help.to_string(),
                None => format!("No help for /{clean}. Type /help for the full list."),
            }
        }
        None => help_text().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::slash::registry;

    /// `/history` and `/sessions` are **not** interchangeable, and the help
    /// must not claim they are. They share one `SessionAction` in the headless
    /// REPL, but `tui/dispatch.rs` intercepts the literal line `/sessions`
    /// before the parser and opens the interactive picker, so in the TUI —
    /// the primary surface — `/sessions` is a picker and `/history` is a text
    /// list. Help that called one an alias of the other would be wrong exactly
    /// where most people read it. Both name "saved sessions" so neither implies
    /// the conversation or the input-line history.
    #[test]
    fn history_and_sessions_help_describe_their_real_surfaces() {
        let history = command_help("history").expect("history has help");
        let sessions = command_help("sessions").expect("sessions has help");

        // Neither may claim to be an alias of the other: the TUI behaviours differ
        assert!(
            !history.contains("alias") && !sessions.contains("alias"),
            "neither may claim aliasing — the TUI routes them differently: {history} / {sessions}"
        );
        // `/sessions` must mention the picker, which is what the TUI does.
        assert!(
            sessions.contains("picker"),
            "`/sessions` help must mention the picker it opens in the TUI, got: {sessions}"
        );
        // Both are about sessions saved on disk, not the conversation and not
        // the input-line history.
        assert!(
            history.contains("saved") && sessions.contains("saved"),
            "both must name saved sessions, got: {history} / {sessions}"
        );
    }

    /// transcript; "conversation and context" is accurate and unambiguous.
    #[test]
    fn clear_help_describes_the_conversation_not_history() {
        let clear = command_help("clear").expect("clear has help");
        assert!(
            clear.contains("conversation") && clear.contains("context"),
            "/clear help should describe the conversation and context, got: {clear}"
        );
        assert!(
            !clear.contains("history"),
            "/clear help must not reuse the overloaded 'history' word (now = saved sessions), got: {clear}"
        );
    }

    /// Today's `/help` listing is a
    /// wall of bare syntax: five lines of commands with no description, and
    /// only the last line says what anything does. This test pins that defect
    /// by name, so the report can show what changed. It asserts the inverse of
    /// what holds today — that every command line in the listing carries a
    /// description — and is therefore red until the listing is rebuilt.
    #[test]
    fn listing_gives_every_command_a_description() {
        let summary = help_text();
        // A command line is one that introduces a slash command (after the
        // indent); a heading or the title is not a command line and need not
        // carry a description. A command line "describes" its command when it
        // pairs the usage with prose via the em-dash separator. Today, the
        // listing is a wall of bare syntax — five lines of commands with no
        // description — so this is red until the listing is rebuilt.
        let bare = summary
            .lines()
            .map(str::trim_start)
            .filter(|line| line.starts_with('/'))
            .filter(|line| !line.contains(" — "))
            .collect::<Vec<_>>();
        assert!(
            bare.is_empty(),
            "every command line should carry a description (— ...), \
             but these are bare syntax with no description:\n{}",
            bare.join("\n")
        );
    }

    /// `/connect` and `/include` sit beside each other in the
    /// listing and must read as a contrast: one replaces the active profile, the
    /// other adds a secondary. A user should be able to tell which is which
    /// without running `/help connect` and `/help include` separately.
    #[test]
    fn connect_and_include_read_as_a_contrast() {
        let summary = help_text();

        let connect_line = summary
            .lines()
            .find(|line| line.trim_start().starts_with("/connect "))
            .unwrap_or_else(|| panic!("listing must have a /connect line, got:\n{summary}"));
        let include_line = summary
            .lines()
            .find(|line| line.trim_start().starts_with("/include "))
            .unwrap_or_else(|| panic!("listing must have a /include line, got:\n{summary}"));

        // The two lines must each state their role. "Replace" names the
        // replacing-the-active-profile behaviour of /connect; "secondary" names
        // the adds-a-profile behaviour of /include. Reading both, a user knows
        // one swaps the active profile and the other layers a secondary on.
        assert!(
            connect_line.to_lowercase().contains("replace"),
            "/connect listing must say it replaces the active profile, got: {connect_line}"
        );
        assert!(
            include_line.to_lowercase().contains("secondary"),
            "/include listing must say it adds a secondary profile, got: {include_line}"
        );
    }

    /// There is one source of description text. The popup in
    /// `complete.rs` and the `/help` listing here must not be two hand-maintained
    /// copies. The popup reads its descriptions from this module's
    /// [`COMMAND_DESCRIPTIONS`]; this test proves that single source covers
    /// exactly the parser's registry, so a command added to one surface cannot
    /// be missing from the other. (The same agreement is checked from the popup
    /// side in `complete.rs::descriptions_cover_exactly_the_registry`, which
    /// reads the same shared table.)
    #[test]
    fn command_descriptions_cover_exactly_the_registry() {
        assert_eq!(
            COMMAND_DESCRIPTIONS.len(),
            registry::KNOWN_COMMANDS.len(),
            "the description table and the command registry must list the same commands"
        );
        for (name, _) in COMMAND_DESCRIPTIONS {
            assert!(
                registry::KNOWN_COMMANDS.contains(name),
                "{name} is described but not in the registry"
            );
        }
        for name in registry::KNOWN_COMMANDS {
            assert!(
                description_for(name).is_some(),
                "{name} is registered but has no description"
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
}
