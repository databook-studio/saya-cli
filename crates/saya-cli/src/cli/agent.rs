//! The two agent-shaped job commands a script also runs: `saya start` and
//! `saya follow-up`. Each states its request plus the repository and the
//! trusted program directory every coding launch carries.

use super::{CodingCommand, ThemeArg};
use clap::Args;
use std::path::PathBuf;

/// Start a job using the configured provider and owned
/// checkouts.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct StartCommand {
    /// Original repository to snapshot; never a child-provided path.
    #[arg(long, default_value = ".")]
    pub repository: PathBuf,
    /// Trusted directory containing installed programs for confined commands.
    #[arg(long, default_value = "/usr/bin")]
    pub program_dir: PathBuf,
    /// Exact durable job id; use a new id for a new task.
    pub id: String,
    /// The task to give the agent.
    pub task: String,
    /// Repository-relative paths this job may edit (repeatable; `.` covers all).
    #[arg(long = "write")]
    pub write_paths: Vec<String>,
    /// Existing database profiles for read-only queries (repeatable).
    #[arg(long = "database-profile")]
    pub database_profiles: Vec<String>,
    /// Start plan-first: read tools only until an operator approves a plan.
    #[arg(long)]
    pub plan: bool,
    /// Explicitly preauthorize this exact task and the stated write scope.
    #[arg(long)]
    pub approve: bool,
    /// Edit this folder in place even when `[job] root_mode = "managed"`.
    #[arg(long = "in-place")]
    pub in_place: bool,
    /// Create a managed checkout instead of editing this folder in place.
    #[arg(long, conflicts_with = "in_place")]
    pub managed: bool,
    /// Accepted for compatibility; the TUI owns the palette.
    #[arg(long, value_enum, hide = true, global = true)]
    pub theme: Option<ThemeArg>,
    /// Accepted for compatibility; the TUI owns the transcript.
    #[arg(long, hide = true, global = true)]
    pub show_thinking: bool,
}

impl StartCommand {
    /// The request a coding launch is composed from.
    ///
    /// The tool list is empty on purpose: `--tools` is the global flag, and
    /// the launch resolves it into the runtime before the job starts, so
    /// nothing is stated per subcommand. The session-only spelling, which has
    /// no launch to resolve into, fills the same field through its own parser.
    pub fn into_coding(self) -> CodingCommand {
        CodingCommand::Start {
            id: self.id,
            task: self.task,
            write_paths: self.write_paths,
            database_profiles: self.database_profiles,
            tools: Vec::new(),
            plan: self.plan,
            approve: self.approve,
            in_place: self.in_place,
            managed: self.managed,
        }
    }

    /// The request and the two paths a coding launch is composed from. The
    /// root mode rides on the command itself.
    pub fn into_parts(self) -> (CodingCommand, PathBuf, PathBuf) {
        let repository = self.repository.clone();
        let program_dir = self.program_dir.clone();
        (self.into_coding(), repository, program_dir)
    }
}

/// Run a bounded new invocation of a completed child in its original owned
/// checkout.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct FollowUpCommand {
    /// Original repository to snapshot; never a child-provided path.
    #[arg(long, default_value = ".")]
    pub repository: PathBuf,
    /// Trusted directory containing installed programs for confined commands.
    #[arg(long, default_value = "/usr/bin")]
    pub program_dir: PathBuf,
    /// Durable job id.
    pub id: String,
    /// Existing child agent id.
    pub agent: String,
    /// Exact predecessor completion sequence, printed by events/notes/status.
    #[arg(long)]
    pub expected_completion_sequence: u64,
    /// Untrusted repair context; at most 4096 bytes, never a capability grant.
    pub instruction: String,
    /// Renew the job's exact current scope for this explicit follow-up.
    #[arg(long)]
    pub approve: bool,
    /// Accepted and ignored: a follow-up follows the job's recorded root mode.
    #[arg(long = "in-place")]
    pub in_place: bool,
    /// Accepted and ignored: a follow-up follows the job's recorded mode.
    #[arg(long, conflicts_with = "in_place")]
    pub managed: bool,
}

impl FollowUpCommand {
    /// The request a follow-up is composed from.
    pub fn into_coding(self) -> CodingCommand {
        CodingCommand::FollowUp {
            id: self.id,
            agent: self.agent,
            expected_completion_sequence: self.expected_completion_sequence,
            instruction: self.instruction,
            approve: self.approve,
            in_place: self.in_place,
            managed: self.managed,
        }
    }

    /// The request and the two paths a coding launch is composed from. The
    /// root mode rides on the command itself.
    pub fn into_parts(self) -> (CodingCommand, PathBuf, PathBuf) {
        let repository = self.repository.clone();
        let program_dir = self.program_dir.clone();
        (self.into_coding(), repository, program_dir)
    }
}
