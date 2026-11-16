use clap::Subcommand;
/// The verbs one coding launch takes, shared by `saya job` and the
/// interactive session.
///
/// The eight agent-shaped controls are hidden from help: they are never a
/// lifecycle verb, so `saya job` must not offer them, and on the command line
/// they have never done anything but answer `NotOwned` — the interactive
/// session owns them. `saya job` refuses them by name (`cli/job.rs`), and
/// `saya start` / `saya follow-up` state the two a script runs.
#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum CodingCommand {
    /// Export bounded owner receipts; missing or unknown evidence remains explicit.
    Export {
        /// Durable job id.
        id: String,
        /// Exclusive durable event cursor; repeat until next_cursor is absent.
        #[arg(long)]
        after: Option<u64>,
        /// Exclusive physical transport reservation cursor.
        #[arg(long)]
        transport_after: Option<u64>,
    },

    /// Read a bounded page of authoritative job events.
    Events {
        /// Durable job id.
        id: String,
        /// Exclusive event sequence cursor.
        #[arg(long)]
        after: Option<u64>,
    },

    /// Build an isolated integration candidate for review.
    Review {
        /// Durable job id.
        id: String,
        /// Fresh candidate checkout id.
        candidate: String,
        /// Renew the job's exact scope for this review.
        #[arg(long)]
        approve: bool,
    },
    /// Review an existing owned candidate after manual edits without rebuilding it.
    ReviewExisting {
        /// Durable job id.
        id: String,
        /// Existing candidate checkout id.
        candidate: String,
        /// Renew the job's exact scope for this review.
        #[arg(long)]
        approve: bool,
    },
    /// Apply a reviewed candidate only if its digest and original tree are fresh.
    Apply {
        /// Durable job id.
        id: String,
        /// Candidate checkout id printed by review.
        candidate: String,
        /// Exact source SHA-256 printed by review.
        #[arg(long)]
        source_sha256: String,
        /// Explicitly approve guarded apply-back.
        #[arg(long)]
        approve: bool,
    },
    /// Remove only an unchanged owned checkout after quiescence and recovery checks.
    RemoveCheckout {
        /// Durable job id.
        id: String,
        /// Owned checkout id.
        checkout: String,
        /// Explicitly approve this removal.
        #[arg(long)]
        approve: bool,
    },
    /// Reconcile an interrupted owned checkout operation without replaying ambiguous effects.
    ReconcileCheckout {
        /// Durable job id.
        id: String,
        /// Owned checkout id.
        checkout: String,
        /// Explicitly approve reconciliation.
        #[arg(long)]
        approve: bool,
    },
    /// Verify an owned checkout stays recoverable; nothing is deleted.
    RetainCheckout {
        /// Durable job id.
        id: String,
        /// Owned checkout id.
        checkout: String,
        /// Explicitly approve this retention.
        #[arg(long)]
        approve: bool,
    },

    /// Resume the preserved job with fresh scope consent and unchanged budget ceilings.
    Resume {
        /// Durable job id.
        id: String,
        /// Explicitly renew the original task's exact stored scope.
        #[arg(long)]
        approve: bool,
    },
    /// Inspect pending and accepted child findings as task-scoped notes.
    Notes {
        /// Durable job id.
        id: String,
    },
    /// Explicitly accept one durable child finding; this grants no permission.
    Accept {
        /// Durable job id.
        id: String,
        /// Completion event sequence.
        sequence: u64,
    },
    /// Promote explicitly selected accepted guidance through project preferences.
    Promote {
        /// Durable job id.
        id: String,
        /// Accepted completion event sequence.
        sequence: u64,
        /// Exact guidance to persist, selected by the user.
        text: String,
        /// Explicitly confirm this permanent project preference.
        #[arg(long)]
        approve: bool,
    },

    /// Approve the exact pending job scope shown by this interactive session.
    #[command(hide = true)]
    Approve {
        /// Pending job id.
        id: String,
    },
    /// Refuse the exact pending job scope shown by this interactive session.
    #[command(hide = true)]
    Deny {
        /// Pending job id.
        id: String,
    },
    /// Run a bounded new invocation of a completed child in its original owned checkout.
    #[command(hide = true)]
    FollowUp {
        /// Durable job id.
        id: String,
        /// Existing child agent id.
        agent: String,
        /// Exact predecessor completion sequence, printed by events/notes/status.
        #[arg(long)]
        expected_completion_sequence: u64,
        /// Untrusted repair context; at most 4096 bytes, never a capability grant.
        instruction: String,
        /// Renew the job's exact current scope for this explicit follow-up.
        #[arg(long)]
        approve: bool,
        /// Edit this folder in place even when the configured default is
        /// managed: tool edits are captured as before-images and no checkout
        /// is created.
        #[arg(long = "in-place")]
        in_place: bool,
        /// Create a managed checkout instead of editing this folder in place.
        #[arg(long, conflicts_with = "in_place")]
        managed: bool,
    },
    /// Send bounded steering to one child in this session's active job.
    #[command(hide = true)]
    Message {
        /// Active job id.
        id: String,
        /// Child agent id.
        agent: String,
        /// Steering text.
        text: String,
    },
    /// Pause one active child at its next safe boundary.
    #[command(hide = true)]
    Pause {
        /// Active job id.
        id: String,
        /// Child agent id.
        agent: String,
    },
    /// Continue one paused child in this session's active job.
    #[command(hide = true)]
    Continue {
        /// Active job id.
        id: String,
        /// Child agent id.
        agent: String,
    },
    /// Cancel one child, preserving its siblings.
    #[command(hide = true)]
    CancelChild {
        /// Active job id.
        id: String,
        /// Child agent id.
        agent: String,
    },

    /// Start a job using the configured provider and owned checkouts.
    #[command(hide = true)]
    Start {
        /// Exact durable job id; use a new id for a new task.
        id: String,
        /// The task to give the agent.
        task: String,
        /// Repository-relative path or subtree this job may edit (repeatable);
        /// pass `.` to cover the whole repository.
        #[arg(long = "write")]
        write_paths: Vec<String>,
        /// Existing bounded, non-whitespace database profile name for read-only queries (repeatable).
        #[arg(long = "database-profile")]
        database_profiles: Vec<String>,
        /// The session-only `--tools` words a `/start` states, filled by
        /// the session's slash parser.
        ///
        /// Not a CLI argument: the flag has exactly one declaration, the
        /// global `--tools`, so `saya --tools read start …` and `saya
        /// start … --tools read` are the same one list, resolved into
        /// the runtime before the job starts. A session has no launch to
        /// resolve into, so its words ride here and may only narrow the list
        /// the session itself advertises.
        #[arg(skip)]
        tools: Vec<String>,
        /// Start plan-first: the root advertises only the read tools until an
        /// operator approves a plan it submits, and that approval opens the
        /// write surface for the rest of the job. Needs an interactive
        /// operator to answer, and at least one `--write` path to open.
        #[arg(long)]
        plan: bool,
        /// Explicitly preauthorize this exact task and the stated write scope.
        #[arg(long)]
        approve: bool,
        /// Accepted and ignored: a follow-up follows the job's recorded mode.
        #[arg(long = "in-place")]
        in_place: bool,
        /// Accepted and ignored: a follow-up follows the job's recorded mode.
        #[arg(long, conflicts_with = "in_place")]
        managed: bool,
    },
    /// List durable jobs for the selected repository.
    List {
        /// Continue after this job id; at most 100 jobs per page.
        #[arg(long)]
        after: Option<String>,
    },
    /// Show a job and its bounded child states.
    Show {
        /// Durable job id.
        id: String,
    },
    /// Record a cancellation request; the live owner cancels and joins its children.
    Cancel {
        /// Durable job id.
        id: String,
    },
    /// Inspect model, Git, scope and confinement readiness without a database.
    Doctor {
        /// Explicitly call the configured provider with a bounded inert native-tool probe.
        #[arg(long)]
        probe: bool,
    },
    /// Check job readiness step by step; installs and enables nothing.
    Setup {
        /// Explicitly call the configured provider with a bounded inert native-tool probe.
        #[arg(long)]
        probe: bool,
    },
    /// List or record what blocks an interrupted job from resuming.
    Reconcile {
        /// Durable job id.
        id: String,
        /// Exact plan digest printed by the listing; with --approve, reconciles exactly that list.
        #[arg(long)]
        plan_sha256: Option<String>,
        /// Explicitly approve reconciling the listed effects.
        #[arg(long)]
        approve: bool,
    },
    /// List retained managed checkouts belonging to a job.
    Worktrees {
        /// Durable job id.
        id: String,
    },

    /// List a job's working-checkout checkpoints, or restore the checkout to one.
    Rewind {
        /// Durable job id.
        id: String,
        /// Checkpoint turn to restore, as the listing numbers them; omit to
        /// list them.
        #[arg(long = "to", value_name = "TURN")]
        to: Option<u32>,
        /// The same turn stated positionally, the `/job rewind <id> <turn>`
        /// spelling. State it once: with both spellings the request is refused.
        #[arg(value_name = "TURN")]
        turn: Option<u32>,
        /// Restore the job's captured baseline instead of a checkpoint: every
        /// baseline file in the job's write scope is put back and paths added
        /// after it are removed.
        #[arg(long)]
        baseline: bool,
        /// Touch a path the job's own journal does not account for. Without
        /// it, a concurrent edit is reported and left as it stands.
        #[arg(long)]
        force: bool,
    },
}

impl CodingCommand {
    /// The job id a lifecycle verb names, or `None` for a verb that names
    /// none (a listing, a readiness check).
    pub fn job_id(&self) -> Option<&str> {
        match self {
            Self::Export { id, .. }
            | Self::Events { id, .. }
            | Self::Review { id, .. }
            | Self::ReviewExisting { id, .. }
            | Self::Apply { id, .. }
            | Self::RemoveCheckout { id, .. }
            | Self::ReconcileCheckout { id, .. }
            | Self::RetainCheckout { id, .. }
            | Self::Resume { id, .. }
            | Self::Notes { id, .. }
            | Self::Accept { id, .. }
            | Self::Promote { id, .. }
            | Self::Approve { id }
            | Self::Deny { id }
            | Self::FollowUp { id, .. }
            | Self::Message { id, .. }
            | Self::Pause { id, .. }
            | Self::Continue { id, .. }
            | Self::CancelChild { id, .. }
            | Self::Start { id, .. }
            | Self::Show { id }
            | Self::Cancel { id }
            | Self::Reconcile { id, .. }
            | Self::Worktrees { id }
            | Self::Rewind { id, .. } => Some(id),
            Self::List { .. } | Self::Doctor { .. } | Self::Setup { .. } => None,
        }
    }
}
