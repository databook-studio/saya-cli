/// The surface a scope list is stated on: one grammar, parsed by one
/// parser; the surface selects which [`NOT_YET_WIRED`] refusals apply. A
/// run's `--allow` and a session's `/allow` state the same words but bind
/// different things — a run approves [`Capabilities`], a session seeds its
/// grant store — so a family can be wired on one and not the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Surface {
    /// `--allow` on a headless run.
    Run,
    /// `/allow` in an interactive session.
    Session,
}

/// The scope grammar, for the error message that names what was refused.
const KNOWN: &str = "known scopes: none, workspace-write, scratch, \
                     fetch:<scheme>+<host>, runner:<program>, interpreter:<program>, \
                     command:<program>, endpoint:<role>=<endpoint>, sql:<connection>";

/// One family-surface refusal: the family parses under the grammar but the
/// named surface binds nothing for it, so approving one there would gate
/// nothing.
struct NotYetWired {
    family: &'static str,
    surface: Surface,
    /// The user-facing reason — wiring, never an absence claim.
    reason: &'static str,
    /// The phrase the tests pin, so the reason cannot drift into a lie.
    /// Read by tests only; production reads `reason`.
    #[cfg_attr(not(test), allow(dead_code))]
    pinned: &'static str,
}

/// Scopes the grammar accepts but a surface cannot yet act on: no tool the
/// surface's decider consults consumes them, so approving one there would
/// gate nothing.
///
/// They are refused at parse time rather than accepted and ignored. The
/// repo's own standard is that a flag implying a capability is available is
/// worse than no flag — a user who types `--allow scratch` and gets a run
/// has been told the model may use a scratch database, and it cannot. An
/// independent review found exactly this shipped, and this list is the fix.
///
/// Entries are per surface by design: `endpoint:` is refused on both, each
/// with its own true reason (a run's episodes and a session's binding are
/// different facts). `sql:` left the list with U4: the run's decider is the
/// frozen session policy seeded from `--allow` (`assembly.rs`), so the
/// stated token gates the SQL family's asks on the connection it names —
/// its consumer exists, on both surfaces. Deleting an entry is the whole of
/// "turning the scope on" once its consumer lands.
const NOT_YET_WIRED: &[NotYetWired] = &[
    NotYetWired {
        family: "endpoint",
        surface: Surface::Run,
        reason: "every episode calls the orchestrator endpoint; per-step roles are not bound yet",
        pinned: "per-step roles are not bound",
    },
    NotYetWired {
        family: "endpoint",
        surface: Surface::Session,
        reason: "a session binds no per-step endpoint roles; a run plan is where roles bind",
        pinned: "binds no per-step endpoint roles",
    },
];

/// A run-surface policy refusal that is permanent by design: the run is
/// unattended and this scope names unconfined host execution. Not a
/// `NOT_YET_WIRED` entry — that list's discipline is wiring-shaped and its
/// entries leave when their consumer lands; this one never leaves. Its own
/// pinned reason, and never an absence claim.
///
/// Read by tests only; production reads `reason`.
#[cfg_attr(not(test), allow(dead_code))]
const RUN_COMMAND_REFUSAL_PIN: &str = "not available on runs, by design";

/// The run-surface refusal for `--deny`: deny is session-shaped, and a run's
/// programs are pre-declared scopes. Its own pinned wording, in the same
/// permanent class as the `command:` refusal above. Read by the run entry
/// (`start.rs`) and `app`'s launch-flag guard; the test seam below pins its
/// bytes.
pub(crate) fn refuse_deny_on_run() -> String {
    crate::interactive::session::permissions::deny::run_surface_refusal()
}

/// The test seam: the run-surface `--deny` refusal's bytes.
#[cfg(test)]
pub(crate) fn refuse_deny_on_run_for_tests() -> String {
    refuse_deny_on_run()
}

/// The run-surface refusal for a `command:` scope: a permanent policy
/// refusal in its own class.
fn run_command_refusal(token: &str) -> String {
    format!(
        "scope `{token}` is not available on runs, by design: a run is unattended and this \
         scope names unconfined host execution; state a sandboxed scope instead (`runner:`, \
         `interpreter:`). Re-run without it."
    )
}

/// The tail that names the surface's own next step. A run re-runs the
/// command without the scope; a slash command has nothing to re-run — the
/// user re-issues `/allow`. The run surface's bytes are pinned by
/// `the_refusal_s_tail_names_the_surface_s_own_next_step`.
fn refusal_tail(surface: Surface) -> &'static str {
    match surface {
        Surface::Run => "Re-run without it.",
        Surface::Session => "Re-issue /allow without it.",
    }
}

/// The refusal for a scope that parses but binds nothing on `surface`.
fn not_yet_wired(token: &str, family: &str, surface: Surface) -> Option<String> {
    let place = match surface {
        Surface::Run => "in a run",
        Surface::Session => "in this session",
    };
    NOT_YET_WIRED
        .iter()
        .find(|entry| entry.family == family && entry.surface == surface)
        .map(|entry| {
            format!(
                "scope `{token}` is not available yet: {reason}. It parses, but nothing \
                 {place} would consume it, so approving it would gate nothing. {tail}",
                reason = entry.reason,
                tail = refusal_tail(surface),
            )
        })
}
