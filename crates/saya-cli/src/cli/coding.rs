use clap::Subcommand;
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

    /// Read a bounded page of authoritative native job events.
    Events {
        /// Durable job id.
        id: String,
        /// Exclusive event sequence cursor.
        #[arg(long)]
        after: Option<u64>,
    },

    /// Build an isolated integration candidate and run configured checks for review.
    Review {
        /// Durable job id.
        id: String,
        /// Fresh candidate checkout id.
        candidate: String,
        /// Renew the job's exact scope for review and checks.
        #[arg(long)]
        approve: bool,
    },
    /// Apply a reviewed candidate only if its digest, checks and original tree are fresh.
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

    /// Resume the preserved native job with fresh scope consent and unchanged budget ceilings.
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

    /// Approve the exact pending coding scope shown by this interactive session.
    Approve {
        /// Pending job id.
        id: String,
    },
    /// Refuse the exact pending coding scope shown by this interactive session.
    Deny {
        /// Pending job id.
        id: String,
    },
    /// Send bounded steering to one child in this session's active job.
    Message {
        /// Active job id.
        id: String,
        /// Child agent id.
        agent: String,
        /// Steering text.
        text: String,
    },
    /// Pause one active child at its next safe boundary.
    Pause {
        /// Active job id.
        id: String,
        /// Child agent id.
        agent: String,
    },
    /// Continue one paused child in this session's active job.
    Continue {
        /// Active job id.
        id: String,
        /// Child agent id.
        agent: String,
    },
    /// Cancel one child, preserving its siblings.
    CancelChild {
        /// Active job id.
        id: String,
        /// Child agent id.
        agent: String,
    },

    /// Start a native coding job using the configured provider and owned checkouts.
    Start {
        /// Exact durable job id; use a new id for a new task.
        id: String,
        /// The coding task to give the native coordinator.
        task: String,
        /// Candidate-relative path or subtree this job may edit (repeatable).
        #[arg(long = "write")]
        write_paths: Vec<String>,
        /// Bare installed program the confined children may run (repeatable).
        #[arg(long = "program")]
        programs: Vec<String>,
        /// Explicitly preauthorize this exact task and the stated write/program scopes.
        #[arg(long)]
        approve: bool,
    },
    /// List durable native jobs for the selected repository.
    List {
        /// Continue after this job id; at most 100 jobs per page.
        #[arg(long)]
        after: Option<String>,
    },
    /// Show a native job and its bounded child states.
    Show {
        /// Durable job id.
        id: String,
    },
    /// Record a cancellation request; the live owner cancels and joins its children.
    Cancel {
        /// Durable job id.
        id: String,
    },
    /// Inspect coding model, Git, scope and confinement readiness without a database.
    Doctor {
        /// Explicitly call the configured provider with a bounded inert native-tool probe.
        #[arg(long)]
        probe: bool,
    },
    /// List retained managed checkouts belonging to a native job.
    Worktrees {
        /// Durable job id.
        id: String,
    },
}
