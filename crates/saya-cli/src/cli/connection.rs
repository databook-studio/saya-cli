use super::{ClaimKindArg, ForgetReasonArg, ReviewDecisionArg};
use clap::Subcommand;

#[derive(Debug, Clone, Subcommand)]
pub enum ConnectionCommand {
    /// List configured connection profiles.
    List,
    /// Connect to a profile and report success/latency.
    Test {
        /// Name of the connection profile to test.
        #[arg(value_name = "PROFILE")]
        profile_name: String,
    },
    /// Print a profile's cached schema, or re-discover it live with `--refresh`.
    Schema {
        /// Name of the connection profile whose schema to read.
        #[arg(value_name = "PROFILE")]
        profile_name: String,
        /// Re-discover the schema live and refresh the cache, instead of reading
        /// the cached copy.
        #[arg(long)]
        refresh: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum ContractsCommand {
    /// List every recalled contract (claim) for a profile.
    List {
        /// Profile whose claims to list; defaults to the active profile.
        #[arg(long)]
        profile: Option<String>,
    },
    /// Show one object's contract — every claim saya recalls for that table.
    Show {
        /// The object to show, as `catalog.schema.table` (or `schema.table`).
        table: String,
        /// Profile whose claim on this object to show; defaults to the active
        /// profile.
        #[arg(long)]
        profile: Option<String>,
    },
    /// List candidate claims awaiting your review (the worklist, not the
    /// archive).
    Queue {
        /// Profile whose candidates to queue; defaults to the active profile.
        #[arg(long)]
        profile: Option<String>,
        /// Maximum candidates to list. Clamped to 200; a queue is a worklist,
        /// not an archive.
        #[arg(long)]
        limit: Option<usize>,
    },
    /// Store a confirmed claim about a table, so recall surfaces it on later
    /// turns. For directive kinds, `--reason` records why it holds.
    Remember {
        /// The object the claim is about, as `catalog.schema.table`.
        table: String,
        /// Kind of claim to store.
        #[arg(long, value_enum)]
        kind: ClaimKindArg,
        /// The claim's value (a description, an alias, a column name, a role…).
        #[arg(long)]
        value: String,
        /// The column a column-scoped claim is about (for `column-description`
        /// and `column-role` kinds).
        #[arg(long)]
        column: Option<String>,
        /// Why the directive claim holds — a sentence the model reads alongside
        /// the value so a claim that contradicts a plausible schema reading
        /// (use `return_date`, not `rental_date`) loses less often. Forwarded to
        /// the directive kinds only (grain, time-column, column-role); ignored
        /// for description/alias. Optional: a claim with no reason is the default.
        #[arg(long)]
        reason: Option<String>,
        /// Profile to store the claim against; defaults to the active profile.
        #[arg(long)]
        profile: Option<String>,
    },
    /// Confirm or reject a claim, naming it by the short `ki-xxxx` prefix that
    /// `contracts list` prints. A prefix matching no claim, or more than one,
    /// is refused and changes nothing.
    //
    // Implementation note, deliberately not a doc comment: clap prints doc
    // comments verbatim in `--help`, so anything here is user-facing. The
    // prefix resolves against the resolved profile's claims and the decision
    // then reaches the existing confirm/reject/use_candidate_once operations —
    // this is not a second implementation of them.
    Decide {
        /// A leading prefix of a stored claim id. Unambiguous-or-refused: zero
        /// matches or more than one is a typed error that changes nothing.
        prefix: String,
        /// The decision to record: `confirm`, `reject`, or `use-once` (admit
        /// a candidate for one recall without promoting it).
        #[arg(long, value_enum)]
        decision: ReviewDecisionArg,
        /// Profile whose claim to decide; defaults to the active profile.
        #[arg(long)]
        profile: Option<String>,
    },
    /// Confirm every candidate claim in the review queue — the same bounded
    /// set `contracts queue` shows. Every candidate still goes through the
    /// per-item validation `contracts decide --confirm` applies, so a batch is
    /// expected to be a mixture: an item dismissed since it was queued, or one
    /// whose object the cached schema can no longer vet, is refused. Every
    /// approved item and every refusal (with its reason) is reported by claim
    /// id; approved items are never rolled back. Exits 0 when anything was
    /// approved (or nothing was waiting) and 2 when every item was refused.
    /// Without `--yes` the queue is printed and nothing is approved.
    //
    // Implementation note, deliberately not a doc comment: clap prints doc
    // comments verbatim in `--help`, so anything here is user-facing. This
    // variant routes through the same `confirm()` the single-item decide path
    // uses — the batch adds no validation and removes none.
    ApproveAll {
        /// Profile whose queue to approve; defaults to the active profile.
        #[arg(long)]
        profile: Option<String>,
        /// Maximum candidates to approve. Clamped to 200, exactly like
        /// `contracts queue` — this approves the queue you were shown, not the
        /// whole archive.
        #[arg(long)]
        limit: Option<usize>,
        /// Approve. Without it the command prints the queue and approves
        /// nothing (the same deny-by-default the `--non-interactive` approval
        /// policy applies); with it the per-item sweep runs and reports.
        #[arg(long)]
        yes: bool,
    },
    /// Tombstone a claim so recall stops surfacing it.
    Forget {
        /// The stored claim id to forget (the full id, or a `ki-xxxx` prefix
        /// that names one claim).
        claim_id: String,
        /// Why the claim is being forgotten.
        #[arg(long, value_enum, default_value_t = ForgetReasonArg::UserRequest)]
        reason: ForgetReasonArg,
    },
    /// Export the profile's Active claims as a portable `saya.context` file:
    /// the reviewed business context — descriptions, aliases, grains, time
    /// columns, roles, join rules, metrics, notes — without profile identity,
    /// review state, evidence, or credentials. Existing file refused without
    /// `--overwrite`. Review the file before sharing.
    Export {
        /// The path to write the context file to.
        #[arg(value_name = "PATH")]
        path: std::path::PathBuf,
        /// Profile whose claims to export; defaults to the active profile.
        #[arg(long)]
        profile: Option<String>,
        /// Replace the file when it already exists.
        #[arg(long)]
        overwrite: bool,
    },
    /// Import a portable `saya.context` file: every item is validated, mapped
    /// onto this profile's schema, and queued as a Pending candidate for your
    /// review — an imported claim is never active just because the file said
    /// so. Objects that do not resolve against this profile's schema, and
    /// slots held by a different local claim, are reported and not written.
    Import {
        /// The context file to import.
        #[arg(value_name = "PATH")]
        path: std::path::PathBuf,
        /// Profile to import against; defaults to the active profile.
        #[arg(long)]
        profile: Option<String>,
        /// Show what the import would do without writing anything.
        #[arg(long)]
        preview: bool,
    },
    /// Import reviewed context from a dbt manifest: model and source
    /// descriptions and `relationships` tests become Pending claims on this
    /// profile, the same path `contracts import` takes. Nothing is executed
    /// and nothing outside the manifest is read.
    ImportDbt {
        /// The dbt `manifest.json` to import (schema v10, v11, or v12).
        #[arg(value_name = "MANIFEST")]
        manifest: std::path::PathBuf,
        /// Profile to import against; defaults to the active profile.
        #[arg(long)]
        profile: Option<String>,
        /// Only nodes whose name matches one of these globs (`*`, `?`).
        #[arg(long, value_name = "GLOB")]
        select: Vec<String>,
        /// Show what the import would do without writing anything.
        #[arg(long)]
        preview: bool,
    },
}
