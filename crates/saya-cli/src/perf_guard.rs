//! The performance guards' written contract: which tools a surface may
//! advertise by default, and how many bytes one tool's description and the
//! database plugin's section may spend.
//!
//! Measured on this tree: unused tool text and long prompts doubled the time a
//! turn took and cost correctness, so a tool is offered on demand, its
//! description is short, and a tool added later is appended at the end of the
//! list rather than inserted. Each list and cap below is today's value plus a
//! small headroom — 5 % for a byte cap, one entry for a count — and every
//! surface's own test asserts against it:
//!
//! - `chat::default_advertisement_tests` — the chat surface
//! - `agent::tools::coding::default_advertisement_tests` — a job and a helper
//! - `plugins::database::section_tests` — the database section
//! - `tests/run_core_parity` — the per-surface request budgets
//!
//! Raising a number here is a reviewed change with a reason, exactly like
//! raising a request budget in `tests/run_core_parity/budget.json`.

/// The names the chat surface may advertise with no evidence in the request
/// and no `[tools] allow` list: today's lean set, the database ask (which is
/// what stands in for the database family until a request shows database
/// work), the one tool that lets a model raise its hand, and the members a
/// later latch — a checkpoint store, a notebook, a background run — appends.
/// The database family itself is *not* here: it is evidence-gated.
pub(crate) const ALWAYS_ON_CHAT: &[&str] = &[
    "use_database",
    "request_clarification",
    "rewind",
    "notebook_edit",
    "checklist",
    "read",
    "glob",
    "grep",
    "edit",
    "write",
    "shell",
    "shell_output",
    "shell_kill",
];

/// The same for a job root: the lean set, its shell family, the two standard
/// read-only database tools it composes, the checkpoint rewind, and the plan
/// gate's one name — a job states `--plan`, so that tool is a fact about the
/// run rather than evidence in its task. `helpers` and the web pair are not
/// here: they are evidence-gated.
pub(crate) const ALWAYS_ON_JOB: &[&str] = &[
    "request_clarification",
    "rewind",
    "submit_plan",
    "notebook_edit",
    "checklist",
    "read",
    "glob",
    "grep",
    "edit",
    "write",
    "shell",
    "shell_output",
    "shell_kill",
    "schema_discovery",
    "bounded_sql_query",
];

/// The same for a helper: the job's set minus `helpers` (only the root
/// delegates), `rewind` (only the job root owns checkpoints) and the plan
/// gate (a helper runs under the job's posture).
pub(crate) const ALWAYS_ON_HELPER: &[&str] = &[
    "request_clarification",
    "notebook_edit",
    "checklist",
    "read",
    "glob",
    "grep",
    "edit",
    "write",
    "shell",
    "shell_output",
    "shell_kill",
    "schema_discovery",
    "bounded_sql_query",
];

/// The longest description one advertised tool may carry, in bytes: today's
/// longest — `shell`, 757 bytes — plus a 5 % headroom.
pub(crate) const TOOL_DESCRIPTION_CAP: usize = 795;

/// The longest the database plugin's section of the system prompt may be, in
/// bytes: today's section with one connection and memory on — 2 660 bytes — plus
/// a 5 % headroom. The section rides every request of a run that has latched the
/// family.
pub(crate) const DB_SECTION_CAP: usize = 2793;

/// Asserts every name `advertised` is on `listed`.
pub(crate) fn assert_only_listed(surface: &str, listed: &[&str], advertised: &[&str]) {
    let unlisted: Vec<&str> = advertised
        .iter()
        .copied()
        .filter(|name| !listed.contains(name))
        .collect();
    assert!(
        unlisted.is_empty(),
        "{surface}'s default advertisement carries {unlisted:?}, which no always-on list allows. A \
         tool is advertised on demand, never by default: gate it behind evidence \
         (`agent::tools::advertisement`) or add its name to its surface's list in \
         `perf_guard.rs` in review."
    );
}

/// Asserts every `(name, description)` pair's description fits the cap.
pub(crate) fn assert_descriptions_within(surface: &str, definitions: &[(String, String)]) {
    for (name, description) in definitions {
        assert!(
            description.len() <= TOOL_DESCRIPTION_CAP,
            "{surface}'s `{name}` description is {} bytes, over the {TOOL_DESCRIPTION_CAP}-byte cap \
             in `perf_guard.rs`. Every request pays for it: shorten the description, or raise the cap \
             there in review with the reason.",
            description.len()
        );
    }
}
