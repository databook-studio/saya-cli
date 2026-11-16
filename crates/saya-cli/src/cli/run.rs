use clap::Subcommand;

#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum RunCommand {
    /// Continue a paused (or crashed) run at its first incomplete step. The
    /// run's journal is the authority; a run with a live holder refuses.
    Resume {
        /// The id of the run to resume (`saya run list` prints them).
        run_id: String,
        /// Retry an incomplete step that may already have produced effects.
        /// Reconcile possible effects before opting in; this is not exactly-once execution.
        #[arg(long)]
        retry_incomplete: bool,
    },
    /// Record a run cancelled. A run with a live holder refuses — cancel the
    /// process that owns it (Ctrl-C) instead; a finished run changes nothing.
    Cancel {
        /// The id of the run to cancel.
        run_id: String,
    },
    /// List every run, most recent first.
    List,
    /// Show one run's status, goal, scopes, pause reason, and the
    /// deliverables its steps recorded.
    Show {
        /// The id of the run to show.
        run_id: String,
    },
    /// Print one run's journal — every lifecycle and step event, in order.
    Log {
        /// The id of the run whose journal to print.
        run_id: String,
    },
}
