use super::DoctorTaskArg;
use clap::Subcommand;

#[derive(Debug, Clone, Subcommand)]
pub enum McpCommand {
    /// Start the stdio MCP server and serve until stdin closes: `initialize`
    /// and `ping` are answered, `tools/list` names the allowed tools, and
    /// `tools/call` runs them under the same read-only gates the CLI uses.
    /// Exits 0 on stdin EOF. Nothing but protocol frames is ever written to
    /// stdout, so a client's transcript stays clean.
    Serve {
        /// Connection profile allowed to this server; repeatable to serve
        /// several. The allowlist is fixed at startup — without any value,
        /// the configured default profile forms it, and with none configured
        /// the server starts empty.
        #[arg(long = "profile", value_name = "P")]
        profile: Vec<String>,
    },
}

#[derive(Debug, Clone, Subcommand)]
pub enum SessionsCommand {
    /// List saved sessions with their sizes.
    List,
    /// Delete one saved session by id (never prompts; refuses a live session).
    Delete {
        /// The id of the saved session to delete.
        #[arg(value_name = "SESSION_ID")]
        id: String,
    },
}

/// Project-preference subcommands: one explicit, bounded CRUD family over
/// per-worktree guidance the user confirmed — never created by the model,
/// never an authority.
#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum PreferencesCommand {
    /// List this worktree's project preferences: explicit, per-worktree
    /// guidance; never permissions.
    List,
    /// Add one preference: explicit, per-worktree guidance; never
    /// permissions.
    Add {
        /// The preference text: plain text the user confirmed, no secrets.
        #[arg(value_name = "TEXT")]
        text: String,
    },
    /// Replace one preference's text: explicit, per-worktree guidance;
    /// never permissions.
    Edit {
        /// The preference id to edit, from `saya preferences list`.
        #[arg(value_name = "ID")]
        id: String,
        /// The replacement text.
        #[arg(value_name = "TEXT")]
        text: String,
    },
    /// Forget one preference: explicit, per-worktree guidance; never
    /// permissions.
    Forget {
        /// The preference id to forget, from `saya preferences list`.
        #[arg(value_name = "ID")]
        id: String,
    },
    /// Forget every preference in this worktree: explicit, per-worktree
    /// guidance; never permissions.
    ForgetAll,
}

#[derive(Debug, Clone, Subcommand)]
pub enum ConfigCommand {
    /// Write starter config.toml and connections.toml templates to your user
    /// config directory (the trusted layer), so a following command does not
    /// warn. Pass `--project` to write this project's untrusted `.saya/` pair
    /// instead — for team-shared, non-secret settings checked into a repo.
    Init {
        /// Write to this project's `.saya/` instead of your user config
        /// directory. The project layer is untrusted, so a command run
        /// afterward warns until you pass `--trust-project-config`.
        #[arg(long)]
        project: bool,
    },
    /// Diagnose configuration: secrets resolve? what provider endpoint is
    /// configured? Exits non-zero (3) when the setup cannot run a query, so a
    /// script can tell.
    Doctor {
        /// Report one task's readiness from the resolved configuration:
        /// query (a database profile), ai (a provider and its key reference),
        /// coding (an AI provider plus a project directory). No network calls.
        #[arg(long, value_enum)]
        task: Option<DoctorTaskArg>,
    },
    /// Print the effective (redacted) configuration as JSON.
    Show {
        /// Also show which layer (default, user file, project file,
        /// environment, command line) supplied each setting. Values are
        /// redacted exactly as without this flag.
        #[arg(long)]
        explain: bool,
    },
}
