use clap::Subcommand;

/// Saved-investigation subcommands. The same enum the slash adapter (S9)
/// will translate into, so the clap surface and the TUI cannot drift apart.
/// Debug is manual (F-5): the run command's `--param` values must never ride
/// diagnostics.
#[derive(Clone, PartialEq, Eq, Subcommand)]
pub enum InvestigationCommand {
    /// Save a bounded read-only SQL query as a portable investigation
    /// document: one JSON file holding the exact SQL plus its name,
    /// description, dialect, connection alias, and referenced tables — no
    /// credentials, rows, results, or machine-specific identity. The SQL is
    /// validated by the same read-only gate execution uses but is never
    /// executed here, and the saving profile is recorded locally as the
    /// review binding. Review the saved SQL before sharing: literals are
    /// stored verbatim.
    Save {
        /// Name of the investigation, shown in `investigation list`.
        #[arg(long)]
        name: String,
        /// Optional description of what the investigation answers.
        #[arg(long)]
        description: Option<String>,
        /// The exact SQL to save. Omit it to read the statement from
        /// `--file`, or from stdin when input is piped in.
        #[arg(long)]
        sql: Option<String>,
        /// Read the SQL from this file instead of `--sql`.
        #[arg(long = "file", value_name = "PATH")]
        file: Option<std::path::PathBuf>,
        /// Connection profile to save against; defaults to the active profile.
        #[arg(long, value_name = "PROFILE")]
        connection: Option<String>,
        /// Declare a parameter the SQL binds by name, as
        /// `name:type[:required]` — type is string, integer, boolean,
        /// decimal, date, or timestamp, and the third part marks it
        /// required (the default is optional). Repeatable: every `:name`
        /// placeholder in the SQL must be declared exactly this way, and
        /// every declaration must appear in the SQL.
        #[arg(long = "param-spec", value_name = "SPEC")]
        param_specs: Vec<String>,
    },
    /// Edit a saved investigation as a new revision: the fields you pass
    /// replace the stored ones and everything else is kept — the id,
    /// dialect, and connection alias never change. The replacement SQL goes
    /// through the same read-only gate saving uses, and the edit leaves the
    /// local review binding on the old revision, so the next run needs
    /// `--revalidate`. At least one field is required.
    Edit {
        /// The id of the investigation to edit (`investigation list` prints
        /// the ids).
        id: String,
        /// A new name for the investigation.
        #[arg(long)]
        name: Option<String>,
        /// A new description, replacing the stored one.
        #[arg(long)]
        description: Option<String>,
        /// The replacement SQL, stored exactly and verbatim. Omit it to read
        /// the statement from `--file`.
        #[arg(long)]
        sql: Option<String>,
        /// Read the replacement SQL from this file instead of `--sql`.
        #[arg(long = "file", value_name = "PATH")]
        file: Option<std::path::PathBuf>,
        /// Replace the whole parameter declaration list with these
        /// `name:type[:required]` specs (same grammar as `save`), checked
        /// against the resulting SQL's `:name` placeholders. Omit it to keep
        /// the stored declarations. Changing the list is a new revision, so
        /// the next run needs `--revalidate`.
        #[arg(long = "param-spec", value_name = "SPEC")]
        param_specs: Vec<String>,
    },
    /// List saved investigations, one line each, at most 50 per page.
    List {
        /// Maximum entries to list, 1-50 (default 50).
        #[arg(long, value_name = "N")]
        limit: Option<usize>,
        /// Skip this many entries before listing.
        #[arg(long, value_name = "N")]
        offset: Option<usize>,
    },
    /// Print one saved investigation's exact definition as JSON, plus the
    /// local review binding for this machine. The opaque profile identity is
    /// never printed.
    Show {
        /// The id of the investigation to show (`investigation list` prints
        /// the ids).
        id: String,
    },
    /// Delete one saved investigation and its local binding. The current
    /// revision is checked first, so a document changed underneath this
    /// command is refused rather than deleted unseen.
    Delete {
        /// The id of the investigation to delete (`investigation list`
        /// prints the ids).
        id: String,
        /// Require the document to be at this revision; omit it to delete
        /// the revision currently on disk.
        #[arg(long, value_name = "N")]
        revision: Option<u32>,
    },
    /// Write one saved investigation's portable definition to a file for
    /// sharing. Only the definition travels: the per-machine local review
    /// binding is never included. The destination must not exist unless
    /// `--overwrite` is passed, and the write is atomic — a refused or
    /// failed export leaves any existing destination unchanged. The exact
    /// SQL is stored verbatim; review it before sharing.
    Export {
        /// The id of the investigation to export (`investigation list`
        /// prints the ids).
        id: String,
        /// Destination file path. An existing file needs `--overwrite`; a
        /// directory or symlink destination is refused.
        #[arg(value_name = "PATH")]
        path: std::path::PathBuf,
        /// Replace an existing destination file. Without it an existing
        /// destination is refused, never clobbered.
        #[arg(long)]
        overwrite: bool,
    },
    /// Read a portable investigation definition file written by `investigation
    /// export`, validate the whole document, print a preview, and store it
    /// with no local connection binding. Nothing is executed and no
    /// connection is made: running it requires an explicit
    /// `investigation run --connection <profile>` later.
    Import {
        /// Path to the definition file to import.
        #[arg(value_name = "PATH")]
        path: std::path::PathBuf,
    },
    /// Replay a saved investigation against an explicit local connection:
    /// the same bounded, read-only query path as `saya query`, with no AI
    /// provider involved. The target comes only from `--connection` or the
    /// stored local review binding — never the active or default profile —
    /// and the review must still match the definition, the target, and the
    /// referenced objects' schema, or the run is refused.
    Run {
        /// The id of the investigation to run (`investigation list` prints
        /// the ids).
        id: String,
        /// Connection profile to replay against. Required on the first run
        /// of an investigation that has no local review binding on this
        /// machine (e.g. one that was imported); the mapping it creates is
        /// remembered so later runs need no flag.
        #[arg(long, value_name = "PROFILE")]
        connection: Option<String>,
        /// Accept the current state and rewrite the local review binding:
        /// use this when a revision, target, or schema change is expected
        /// and reviewed.
        #[arg(long)]
        revalidate: bool,
        /// Write the Markdown report for this replay's result to a file:
        /// the same report the TUI's `/report` writes — SQL and provenance
        /// by default, no rows. Written only after a successful replay; a
        /// refused or failed write reports on stderr and exits 2, never
        /// touching an existing destination without `--overwrite`.
        #[arg(long, value_name = "PATH")]
        report: Option<std::path::PathBuf>,
        /// Include up to this many result rows in the report's table (at
        /// most 100); omit the flag to write SQL and provenance only.
        #[arg(long, value_name = "N")]
        rows: Option<usize>,
        /// Replace an existing report destination. Without it an existing
        /// destination is refused, never clobbered.
        #[arg(long)]
        overwrite: bool,
        /// Bind a declared parameter, as `name=value`. Repeatable; the names
        /// must be the investigation's declared parameters, values parse as
        /// their declared types, and the literal `null` binds a typed null.
        /// Values bind natively at execute time and are never stored.
        #[arg(long = "param", value_name = "NAME=VALUE")]
        params: Vec<String>,
    },
}
