/// The one-line description per command — **the single source**. Keyed by the
/// names in [`crate::slash::registry::KNOWN_COMMANDS`]; a test asserts the two
/// never drift apart. `interactive::tui::complete` reads this for its popup, and
/// the `/help` listing reads it here, so the two surfaces share one copy.
///
/// `/connect` and `/include` carry the contrast (one replaces, one adds a
/// secondary) so a user reading the listing can tell them apart without two more
/// `/help <name>` round-trips.
///
/// The bypass-composition sentence, shared verbatim by the `/mode` help entry
/// and both `/mode` answers: under `bypass`, Plan still denies writes. One
/// constant so the surfaces state it in the same words; tests assert each
/// surface contains it verbatim. Kept beside the command it documents.
pub(crate) const PLAN_BYPASS_SENTENCE: &str = "Plan still denies writes under `bypass`";
pub(crate) const COMMAND_DESCRIPTIONS: &[(&str, &str)] = &[
    ("coding", "Start and control bounded native coding jobs"),
    ("connect", "Replace the active database profile"),
    ("connections", "List configured database connections"),
    ("include", "Add a secondary database profile to query scope"),
    ("exclude", "Remove a database profile from query scope"),
    ("provider", "Set or view the AI provider"),
    ("model", "Set or view the AI model"),
    ("privacy", "Enable or disable data sharing privacy"),
    ("approvals", "Set approval policy for tool execution"),
    ("mode", "Set the agent's task posture: build or plan"),
    ("schema", "Inspect or refresh database schema"),
    ("doctor", "Diagnose config: secrets, provider endpoint"),
    ("usage", "Show session token usage and cache hit rate"),
    ("workspace", "Show the session's bound workspace root"),
    ("thinking", "Toggle display of the model's chain-of-thought"),
    (
        "tasks",
        "Show the session's task list, or clear it with /tasks clear",
    ),
    (
        "notes",
        "Your notes on this session's saved summary: show, set, clear; separate from /preferences and /remember",
    ),
    (
        "recovery",
        "Record file edits so you can undo them; list, preview and restore",
    ),
    ("sql", "Run a raw SQL query against the active profile"),
    (
        "export",
        "Export the last query's fresh result, or the captured /sql result with --snapshot",
    ),
    (
        "report",
        "Write a Markdown report (SQL + provenance) of the captured /sql result",
    ),
    (
        "investigation",
        "Save, list, show, run, export, import, or delete a saved investigation",
    ),
    (
        "investigations",
        "Open a picker over saved investigations (TUI); /investigation list prints",
    ),
    (
        "chart",
        "Re-run the last query and render the fresh result as an HTML chart",
    ),
    ("explain", "Explain the given or last SQL statement"),
    ("clear", "Clear current session context"),
    ("compact", "Summarise older turns to shrink working memory"),
    ("history", "List saved sessions as text"),
    (
        "sessions",
        "Browse saved sessions: list or delete; bare opens a picker in the TUI",
    ),
    (
        "preferences",
        "Explicit per-worktree guidance you confirmed: list, add, edit, forget; \
         never permissions; separate from /remember",
    ),
    ("resume", "Resume a saved session by id"),
    (
        "rotate",
        "Continue in a new, smaller session that keeps your summary and notes (run /compact first)",
    ),
    ("setup", "Run the guided setup (full-screen terminal only)"),
    (
        "reload",
        "Re-read the configuration between turns: applies what is safe now, says what needs a new session. Data sharing and host commands never change in a running session.",
    ),
    ("columns", "Choose which columns wide result tables show"),
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
    ("run", "Start or operate a headless run from the session"),
    ("runs", "List runs, or show one run by id"),
    (
        "allow",
        "Seed pre-authorised scopes into this session's grant store",
    ),
    (
        "context",
        "Show the instruction sources loaded into requests, never their text",
    ),
    ("grants", "List the session's granted scopes"),
    ("help", "Show help for slash commands"),
    ("exit", "Exit the REPL"),
    ("quit", "Exit the REPL"),
];
