//! One catalogue of every slash command — the single source of truth.
//!
//! The parser stays a `match` in `slash/dispatch.rs` (a table lookup would
//! cost the per-command argument grammars their types), but every other list
//! reads this table: [`crate::slash::registry::closest_command`] for the
//! "did you mean" suggestion, `/help`'s grouped listing and per-name
//! description, the completion popup (`interactive::tui::complete`), and the
//! refusal of a user command file that would shadow a built-in.
//!
//! Today the table records **today's** names and behavior only: one row per
//! canonical name, with the spellings it answers to folded in as aliases. The
//! de-duplication to the 34 canonical names happens in later units; this unit
//! only removes the four hand-maintained lists that could drift.
//!
//! A test reads the parser's arms and asserts they and the catalogue name the
//! same commands, in both directions, so adding an arm without a row (or the
//! reverse) fails the build.

use Category::{Connections, Jobs, Memory, Model, Query, Runs, Session};

const BOTH: Surfaces = Surfaces::BOTH;
const TUI: Surfaces = Surfaces::TUI;

/// The `/help` heading a command is listed under, in heading order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Category {
    Jobs,
    Connections,
    Model,
    Query,
    Session,
    Memory,
    Runs,
}

impl Category {
    /// The headings of the `/help` listing, in the order it prints them.
    pub(crate) const ALL: [Category; 7] = [
        Category::Jobs,
        Category::Connections,
        Category::Model,
        Category::Query,
        Category::Session,
        Category::Memory,
        Category::Runs,
    ];

    /// The heading this category prints.
    pub(crate) const fn heading(self) -> &'static str {
        match self {
            Category::Jobs => "Jobs",
            Category::Connections => "Connections",
            Category::Model => "Provider, model & privacy",
            Category::Query => "Query & data",
            Category::Session => "Session",
            Category::Memory => "Memory",
            Category::Runs => "Runs",
        }
    }
}

/// A surface that runs slash commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Surface {
    /// The line REPL. Its reader lands with the line adapter, so today only
    /// the popup's [`Surface::Tui`] lookup is a production call site.
    #[allow(dead_code)]
    Line,
    Tui,
}

/// The surfaces one command's behavior exists on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Surfaces {
    line: bool,
    tui: bool,
}

impl Surfaces {
    /// The command behaves on the line REPL and in the TUI alike.
    pub(crate) const BOTH: Surfaces = Surfaces {
        line: true,
        tui: true,
    };

    /// The command needs the full-screen terminal; the line REPL refuses it.
    pub(crate) const TUI: Surfaces = Surfaces {
        line: false,
        tui: true,
    };

    /// Whether the command is available on `surface`.
    pub(crate) const fn covers(self, surface: Surface) -> bool {
        match surface {
            Surface::Line => self.line,
            Surface::Tui => self.tui,
        }
    }
}

/// Another spelling of a command, with its own one-line description.
pub(crate) struct Alias {
    pub(crate) name: &'static str,
    pub(crate) description: &'static str,
}

/// One slash command.
pub(crate) struct CommandSpec {
    /// The canonical spelling, without the `/`.
    pub(crate) name: &'static str,
    /// The other spellings that resolve to this command.
    pub(crate) aliases: &'static [Alias],
    /// The one-line description: the popup row and the `/help` listing line.
    pub(crate) description: &'static str,
    /// The `/help` heading it is listed under.
    pub(crate) category: Category,
    /// The surfaces its behavior exists on.
    pub(crate) surfaces: Surfaces,
    /// The `/help` listing: the usage form and its rank in the listing, which
    /// is the listing's print order. `None` for a name the listing folds
    /// under another command.
    pub(crate) listed: Option<(&'static str, u8)>,
}

impl CommandSpec {
    /// Every name this row answers to: the canonical name, then its aliases.
    pub(crate) fn names(&self) -> impl Iterator<Item = &'static str> {
        std::iter::once(self.name).chain(self.aliases.iter().map(|alias| alias.name))
    }
}

/// A row with no alias spelling.
const fn cmd(
    name: &'static str,
    category: Category,
    surfaces: Surfaces,
    description: &'static str,
    listed: Option<(&'static str, u8)>,
) -> CommandSpec {
    CommandSpec {
        name,
        aliases: &[],
        description,
        category,
        surfaces,
        listed,
    }
}

/// A row with the other spellings that resolve to it.
const fn aliased(
    name: &'static str,
    category: Category,
    surfaces: Surfaces,
    description: &'static str,
    listed: Option<(&'static str, u8)>,
    aliases: &'static [Alias],
) -> CommandSpec {
    CommandSpec {
        name,
        aliases,
        description,
        category,
        surfaces,
        listed,
    }
}

/// Every slash command, in the order the parser's `match` declares them (the
/// order the completion popup lists them in). `listed` carries the `/help`
/// view: its heading comes from `category`, its line and place from `listed`.
// rustfmt must not reflow the table: one row per two lines is what makes a
// 56-row table reviewable, and the usage strings are read as they print.
#[rustfmt::skip]
pub(crate) const CATALOGUE: &[CommandSpec] = &[
    cmd("start", Jobs, BOTH, "Start a job for long or background work; ordinary work is just a message",
        Some(("/start [--id <id>] <task> [--write path] [--database-profile name] [--tools name,...] [--plan] [--in-place] [--approve]", 1))),
    cmd("follow-up", Jobs, BOTH, "Run a bounded new invocation of a completed child in its checkout",
        Some(("/follow-up <id> <agent> --expected-completion-sequence <n> <instruction> [--approve]", 2))),
    cmd("job", Jobs, BOTH, "Operate a durable job's lifecycle: review, apply, resume, list, show, cancel, and the rest",
        Some(("/job <export|events|review|apply|resume|list|show|cancel|doctor|setup|reconcile|worktrees|rewind> …", 3))),
    cmd("approve", Jobs, BOTH, "Approve the exact pending job scope this session is showing",
        Some(("/approve <id>", 4))),
    cmd("deny", Jobs, BOTH, "Refuse the exact pending job scope this session is showing",
        Some(("/deny <id>", 5))),
    cmd("message", Jobs, BOTH, "Send bounded steering to one child in this session's active job",
        Some(("/message <id> <agent> <text>", 6))),
    cmd("pause", Jobs, BOTH, "Pause one active child at its next safe boundary",
        Some(("/pause <id> <agent>", 7))),
    cmd("continue", Jobs, BOTH, "Continue one paused child in this session's active job",
        Some(("/continue <id> <agent>", 8))),
    cmd("cancel-child", Jobs, BOTH, "Cancel one child, preserving its siblings",
        Some(("/cancel-child <id> <agent>", 9))),
    cmd("connect", Connections, BOTH, "Replace the active database profile",
        Some(("/connect <profile>", 10))),
    cmd("connections", Connections, BOTH, "List configured database connections",
        Some(("/connections", 11))),
    cmd("include", Connections, BOTH, "Add a secondary database profile to query scope",
        Some(("/include <profile>", 12))),
    cmd("exclude", Connections, BOTH, "Remove a database profile from query scope",
        Some(("/exclude <profile>", 13))),
    cmd("provider", Model, BOTH, "Set or view the AI provider",
        Some(("/provider [name]", 14))),
    cmd("model", Model, BOTH, "Set or view the AI model",
        Some(("/model [name]", 15))),
    cmd("privacy", Model, BOTH, "Enable or disable data sharing privacy",
        Some(("/privacy [on|off]", 16))),
    aliased("mode", Model, BOTH, "Set the session's consent mode: ask, accept-edits, plan, bypass, read-only, or never",
        Some(("/mode [ask|accept-edits|plan|bypass|read-only|never]", 18)),
        &[Alias { name: "approvals", description: "Deprecated alias for /mode (removed next release)" }]),
    cmd("schema", Query, BOTH, "Inspect or refresh database schema",
        Some(("/schema [refresh]", 21))),
    cmd("sql", Query, BOTH, "Run a raw SQL query against the active profile",
        Some(("/sql <query>", 22))),
    cmd("export", Query, BOTH, "Export the last query's fresh result, or the captured /sql result with --snapshot",
        Some(("/export [--snapshot|--refresh] [--overwrite] <path>", 23))),
    cmd("report", Query, BOTH, "Write a Markdown report (SQL + provenance) of the captured /sql result",
        Some(("/report [--rows N] [--overwrite] <path>", 24))),
    aliased("investigation", Query, BOTH, "Save, list, show, run, export, import, or delete a saved investigation",
        Some(("/investigation save|list|show|run|export|import|delete", 25)),
        &[Alias { name: "investigations", description: "Open a picker over saved investigations (TUI); /investigation list prints" }]),
    cmd("chart", Query, BOTH, "Re-run the last query and render the fresh result as an HTML chart",
        Some(("/chart [type] [path]", 26))),
    cmd("explain", Query, BOTH, "Explain the given or last SQL statement",
        Some(("/explain [sql]", 27))),
    cmd("clear", Session, BOTH, "Clear current session context",
        Some(("/clear", 29))),
    cmd("undo", Session, BOTH, "Undo the last turn's file edits: restore the run's checkpoint taken before that turn",
        Some(("/undo", 30))),
    cmd("rewind", Session, BOTH, "List the run's checkpoints, newest first, or restore the one you name",
        Some(("/rewind [<id>]", 31))),
    cmd("compact", Session, BOTH, "Summarise older turns to shrink working memory",
        Some(("/compact", 32))),
    cmd("history", Session, BOTH, "List saved sessions as text",
        Some(("/history", 33))),
    cmd("notes", Session, BOTH, "Your notes on this session's saved summary: show, set, clear; separate from /preferences and /remember",
        Some(("/notes [set <text>|clear]", 45))),
    cmd("sessions", Session, BOTH, "Browse saved sessions: list or delete; bare opens a picker in the TUI",
        Some(("/sessions [list|delete <id>]", 34))),
    cmd("preferences", Memory, BOTH, "Explicit per-worktree guidance you confirmed: list, add, edit, forget; never permissions; separate from /remember",
        Some(("/preferences [list|add <text>|edit <id> <text>|forget <id>|forget-all]", 52))),
    cmd("resume", Session, BOTH, "Resume a saved session by id",
        Some(("/resume <id>", 35))),
    cmd("rotate", Session, BOTH, "Continue in a new, smaller session that keeps your summary and notes (run /compact first)",
        Some(("/rotate", 36))),
    cmd("setup", Session, TUI, "Run the guided setup (full-screen terminal only)",
        Some(("/setup", 37))),
    cmd("reload", Session, BOTH, "Re-read the configuration between turns: applies what is safe now, says what needs a new session. Data sharing and host commands never change in a running session.",
        Some(("/reload", 38))),
    cmd("columns", Query, BOTH, "Choose which columns wide result tables show",
        Some(("/columns [name,name,… | all]", 28))),
    aliased("contracts", Memory, BOTH, "The knowledge store: list every contract, show one object's contract, review and decide the queue, and export or import the context",
        Some(("/contracts <list|show|queue|confirm|reject|confirm-all|export|import>", 49)),
        &[Alias { name: "contract", description: "Deprecated alias for /contracts (removed next release)" },
          Alias { name: "queue", description: "Deprecated alias for /contracts queue (removed next release)" },
          Alias { name: "confirm", description: "Deprecated alias for /contracts confirm (removed next release)" },
          Alias { name: "reject", description: "Deprecated alias for /contracts reject (removed next release)" },
          Alias { name: "approve-all", description: "Deprecated alias for /contracts confirm-all: it confirms knowledge claims, not job approvals (removed next release)" }]),
    cmd("remember", Memory, BOTH, "Store a confirmed contract claim",
        Some(("/remember <table> <kind> <value…>", 50))),
    cmd("forget", Memory, BOTH, "Tombstone a contract claim so recall excludes it",
        Some(("/forget <id>", 51))),
    cmd("run", Runs, BOTH, "Start or operate a headless run from the session",
        Some(("/run <goal…> --allow <scopes>", 55))),
    cmd("runs", Runs, BOTH, "List runs, or show one run by id",
        Some(("/runs [id]", 56))),
    cmd("allow", Model, BOTH, "Seed pre-authorised scopes into this session's grant store",
        Some(("/allow <scopes…>", 19))),
    cmd("context", Session, BOTH, "Show the instruction sources loaded into requests, never their text",
        Some(("/context", 39))),
    cmd("grants", Model, BOTH, "List the session's granted scopes",
        Some(("/grants", 20))),
    cmd("doctor", Session, BOTH, "Diagnose config: secrets, provider endpoint",
        Some(("/doctor", 40))),
    cmd("usage", Session, BOTH, "Show session token usage and cache hit rate",
        Some(("/usage", 41))),
    cmd("workspace", Session, BOTH, "Show the session's bound workspace root",
        Some(("/workspace", 42))),
    cmd("thinking", Session, BOTH, "Toggle display of the model's chain-of-thought",
        Some(("/thinking [on|off]", 43))),
    cmd("tasks", Session, BOTH, "Show the session's task list, or clear it with /tasks clear",
        Some(("/tasks [clear]", 44))),
    cmd("recovery", Session, BOTH, "Record file edits so you can undo them; list, preview and restore",
        Some(("/recovery [on|off|list|diff <id>|restore <id>]", 46))),
    cmd("help", Session, BOTH, "Show help for slash commands",
        Some(("/help [command]", 47))),
    aliased("exit", Session, BOTH, "Exit the REPL",
        Some(("/exit  (alias /quit)", 48)),
        &[Alias { name: "quit", description: "Exit the REPL" }]),
];

include!("catalog/lookup.rs");
