//! `--allow <scopes>` parsing: the CLI side of the run approval.
//!
//! A headless run cannot prompt, so its capability set must be stated before
//! anything starts — and it must be stated exactly: an unknown scope name is
//! a usage error, never a silently ignored token (a run that believes it
//! approved more or less than the user typed is a lying run). The grammar
//! mirrors the [`Capabilities`] contract fields; the built approval is
//! exactly the stated scopes, nothing implicit.

use saya_types::{
    Capabilities, Destination, EndpointBindings, FetchScope, InterpreterScope, RunnerScope,
    is_bare_name, is_name_shaped, is_refused_runner_program,
};

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

/// The scopes the approval carries: the stated tokens, with the fetch
/// family normalised to the URL parser's spelling at parse — the run
/// engine's fetch policy compares the lowercased destination, and the grant
/// token suggester produces the same form from the call's URL, so a seeded
/// token and a suggested token are one string for one destination. A
/// session's `/allow` seeds these words, so `/grants` shows the grammar's
/// form of what was granted. A run's approval is `capabilities` plus the
/// frozen decider's seeds. `command:` tokens build no run capability and
/// ride the tokens verbatim (the run surface never produces them — it
/// refuses them above — so they only flow on the session surface).
#[derive(Debug)]
pub(crate) struct Approved {
    pub(crate) capabilities: Capabilities,
    pub(crate) tokens: Vec<String>,
}

/// The stated tokens that are per-call grant words — the families that bind
/// no run capability (`Capabilities` has no field for them), so they ride
/// the approval's tokens verbatim: the frozen decider seeds them and the
/// journal's `PlanApproved` payload carries them, in stated order, deduped.
/// Today that family is `sql:` only.
pub(crate) fn carried_tokens(tokens: &[String]) -> Vec<String> {
    let mut carried = Vec::new();
    for token in tokens {
        if token.starts_with("sql:") && !carried.contains(token) {
            carried.push(token.clone());
        }
    }
    carried
}

/// Parses the `--allow` tokens. An empty list is the caller's refusal
/// decision, not a silently-empty approval; anything here that does not
/// match the grammar is a typed usage error. `none` is the grammar's
/// explicit empty approval — see the head of the body.
///
/// `pub(crate)` so the interactive session's grant token suggester and the
/// `/allow` command can feed every token they produce or accept back
/// through this one parser — the grammar's authority is here, never a
/// duplicate. The `surface` selects which [`NOT_YET_WIRED`] refusals apply;
/// the grammar itself is one loop, one `KNOWN`, one shape rule set.
pub(crate) fn parse(tokens: &[String], surface: Surface) -> Result<Approved, String> {
    // `none` states the empty approval: no capabilities at all, read-only
    // by construction — the episode's per-tool-call decider already
    // defaults to read-only (`assembly.rs`), and nothing a refused scope
    // would gate is reachable. It must stand alone: beside a scope it
    // would state both "nothing" and "something", which states nothing.
    if tokens.iter().any(|token| token == "none") {
        if tokens.len() > 1 {
            return Err(format!(
                "`none` states the empty scope set and must be the only token; {KNOWN}"
            ));
        }
        return Ok(Approved {
            capabilities: Capabilities::default(),
            tokens: tokens.to_vec(),
        });
    }
    // The grammar's words as the approval carries them, normalised where
    // the engine normalises: a fetch destination's spelling is the URL
    // parser's — lowercased scheme and host — and the run engine's fetch
    // policy compares exactly that form, as does the grant token suggester
    // spelling a call's URL. Normalising at parse, on both surfaces, makes
    // a seeded token and a suggested token one string for one destination:
    // whatever casing the user typed, the grant pre-answers the call it
    // names (U6 defect 2). Every other family rides the stated spelling
    // verbatim: runner programs and connection names are case-sensitive
    // identifiers nothing in the engine folds, so normalising them would
    // widen a grant across distinct names.
    let stated: Vec<String> = tokens
        .iter()
        .map(|token| match token.split_once(':') {
            Some(("fetch", rest)) => format!("fetch:{}", rest.to_ascii_lowercase()),
            _ => token.clone(),
        })
        .collect();
    let mut capabilities = Capabilities::default();
    let mut destinations = Vec::new();
    let mut programs = Vec::new();
    let mut interpreters = Vec::new();
    let mut bindings = Vec::new();
    for token in &stated {
        if token == "workspace-write" {
            capabilities.workspace_write = true;
        } else if token == "scratch" {
            capabilities.scratch = true;
        } else if let Some(rest) = token.strip_prefix("fetch:") {
            let Some((scheme, host)) = rest.split_once('+') else {
                return Err(format!(
                    "scope `{token}` must be fetch:<scheme>+<host>; {KNOWN}"
                ));
            };
            let destination = Destination::new(scheme, host).map_err(|_| {
                format!("scope `{token}` is not a scheme plus a bare host; {KNOWN}")
            })?;
            destinations.push(destination);
        } else if let Some(rest) = token.strip_prefix("runner:") {
            // The mirror, in both directions: a name the runner refuses is
            // the interpreter family's member, not the runner's — a token
            // that approved it here would be a lying scope, refused at call
            // time after surviving the pre-authorization.
            if is_refused_runner_program(rest) {
                return Err(format!(
                    "`{rest}` is a shell or interpreter the runner refuses; use \
                     `interpreter:{rest}` to approve it explicitly; {KNOWN}"
                ));
            }
            programs.push(rest.to_string());
        } else if let Some(rest) = token.strip_prefix("interpreter:") {
            if rest.is_empty() {
                return Err(format!(
                    "scope `{token}` must be interpreter:<program>; {KNOWN}"
                ));
            }
            // The family IS the runner's refusal list, mirrored at parse
            // time: a non-refused name never rides the interpreter family —
            // it belongs to the runner's, which can run it.
            if !is_refused_runner_program(rest) {
                return Err(format!(
                    "`{rest}` is not a shell or interpreter the runner refuses; use \
                     `runner:{rest}` to approve it as a runner program; {KNOWN}"
                ));
            }
            interpreters.push(rest.to_string());
        } else if let Some(rest) = token.strip_prefix("command:") {
            // The host lane's own family: payload a bare name, shells
            // included — the `runner:`/`interpreter:` family mirror
            // deliberately does not apply here (`command:bash` is the token,
            // and the warning rides the prompt). On the run surface this is
            // a permanent policy refusal in its own class, never an absence
            // claim; on the session surface it parses and the composition
            // gate (`allow_refusal`) decides whether this session carries it.
            if !is_bare_name(rest) {
                return Err(format!(
                    "scope `{token}` must be command:<program>, a bare name — never a path \
                     or traversal; {KNOWN}"
                ));
            }
            if surface == Surface::Run {
                return Err(run_command_refusal(token));
            }
        } else if let Some(rest) = token.strip_prefix("endpoint:") {
            // The payload is judged by the grammar's own shape rule first:
            // a refusal message that says "it parses" must be true.
            let Some((role, endpoint)) = rest.split_once('=') else {
                return Err(format!(
                    "scope `{token}` must be endpoint:<role>=<endpoint>; {KNOWN}"
                ));
            };
            if !is_name_shaped(role) || !is_name_shaped(endpoint) {
                return Err(format!(
                    "scope `{token}` must name a role and an endpoint with the shape a \
                     run-scoped name has; {KNOWN}"
                ));
            }
            if let Some(refusal) = not_yet_wired(token, "endpoint", surface) {
                return Err(refusal);
            }
            bindings.push((role.to_string(), endpoint.to_string()));
        } else if let Some(rest) = token.strip_prefix("sql:") {
            // The payload is a connection's registry name, judged by the
            // same name-shape rule `endpoint:` payloads are. The token builds
            // no run capability: it is a per-call grant word, carried on
            // `Approved::tokens` — the run's frozen decider seeds it and the
            // journal records it (U4).
            if !is_name_shaped(rest) {
                return Err(format!(
                    "scope `{token}` must be sql:<connection>, a connection's registry \
                     name; {KNOWN}"
                ));
            }
        } else {
            return Err(format!("unknown scope `{token}`; {KNOWN}"));
        }
    }
    if !destinations.is_empty() {
        capabilities.fetch = Some(
            FetchScope::new(destinations)
                .map_err(|error| format!("fetch scope refused: {error}"))?,
        );
    }
    if !programs.is_empty() {
        capabilities.runner = Some(
            RunnerScope::new(programs).map_err(|error| format!("runner scope refused: {error}"))?,
        );
    }
    if !interpreters.is_empty() {
        capabilities.interpreter = Some(
            InterpreterScope::new(interpreters)
                .map_err(|error| format!("interpreter scope refused: {error}"))?,
        );
    }
    if !bindings.is_empty() {
        capabilities.endpoints = EndpointBindings::new(bindings)
            .map_err(|error| format!("endpoint bindings refused: {error}"))?;
    }
    Ok(Approved {
        capabilities,
        tokens: stated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An independent review found `--allow scratch`, `fetch:`, `runner:` and
    /// `endpoint:` accepted, persisted and rendered while no tool in a run's
    /// universe consumed any of them. Approving a capability that gates
    /// nothing tells the user something false about what the model may do,
    /// so each is refused until its wiring lands. `scratch` wired with its
    /// tool (S1), `fetch` (S2), `runner` (S3) and `sql:` (U4 — the run's
    /// frozen decider consults its seeds) left this list; the rest stay
    /// refused, and this test keeps refusing them the day they are typed.
    #[test]
    fn a_scope_nothing_consumes_is_refused_rather_than_silently_approved() {
        let token = "endpoint:analyst=fast";
        let Err(error) = parse(&[token.to_string()], Surface::Run) else {
            panic!("`{token}` gates nothing and must be refused");
        };
        assert!(
            error.contains("not available yet"),
            "the refusal must say why, got: {error}"
        );
        assert!(
            error.contains(token),
            "the refusal must name the scope, got: {error}"
        );
    }

    /// The reason every entry here is refused is wiring, not absence: each
    /// named capability exists in the run engine, and a surface's tool
    /// universe simply does not consume it yet. An absence claim goes stale
    /// the moment the tool lands — the exact lie `runner:` shipped after
    /// M5-4's `run_program` merged. The list is the single source of the
    /// user-facing reason text, so each entry is pinned to the one phrasing
    /// that is true **for that entry** — a family's reason rides the
    /// surface it is refused on (`endpoint:`'s run reason and session
    /// reason differ) — and a future drift from the pin is a diff in this
    /// test rather than a lie to the user. The "never an absence claim"
    /// assertion applies to every entry, on every surface.
    #[test]
    fn a_refusal_names_the_wiring_reason_never_an_absence_claim() {
        for entry in NOT_YET_WIRED {
            assert!(
                !entry.reason.contains("not exist"),
                "`{}`'s refusal on {:?} claims a tool is absent — the refusal \
                 class is wiring, not absence: {reason}",
                entry.family,
                entry.surface,
                reason = entry.reason
            );
            assert!(
                entry.reason.contains(entry.pinned),
                "`{}`'s refusal on {:?} must state its true reason (the pin) \
                 verbatim, got: {reason}",
                entry.family,
                entry.surface,
                reason = entry.reason
            );
        }
    }

    /// Each surface's refusal entries are distinct facts, so the pins are
    /// per entry and must not collide: the session's `endpoint:` reason is
    /// its own words (what a session binds), never the run's phrase. One
    /// phrase pinned onto two futures is how the run's words ended up
    /// describing a session's refusal — this keeps every phrase its
    /// entry's own.
    #[test]
    fn every_refusal_pin_is_its_entry_s_own_phrase() {
        for entry in NOT_YET_WIRED {
            for other in NOT_YET_WIRED {
                if std::ptr::eq(entry, other) {
                    continue;
                }
                assert!(
                    !other.reason.contains(entry.pinned),
                    "`{}` on {:?} must not carry `{}`'s pinned phrase \
                     ({:?}): {reason}",
                    other.family,
                    other.surface,
                    entry.family,
                    entry.pinned,
                    reason = other.reason
                );
            }
        }
    }

    /// `sql:<connection>` joins the grammar on the session surface: `/allow
    /// sql:analytics` parses and carries the token verbatim — the session's
    /// grant store seeds the words as stated. (U4 put the run surface on the
    /// same engine, so `sql:` is no longer run-refused; the run surface's
    /// inverse pin is
    /// `sql_gates_a_run_s_sql_family_asks`.)
    #[test]
    fn sql_joins_the_grammar_on_the_session_surface() {
        let Ok(approved) = parse(&["sql:analytics".to_string()], Surface::Session) else {
            panic!("`sql:analytics` must parse on the session surface");
        };
        assert!(
            approved.tokens.iter().any(|token| token == "sql:analytics"),
            "the session approval carries the token verbatim: {:?}",
            approved.tokens
        );
        // The grammar names capabilities a run's `Capabilities` carries;
        // `sql:` is a session grant word, so it builds no run capability —
        // the other fields stay at their defaults.
        assert!(!approved.capabilities.workspace_write);
        assert!(!approved.capabilities.scratch);
        assert!(approved.capabilities.fetch.is_none());
        assert!(approved.capabilities.runner.is_none());
        assert!(approved.capabilities.interpreter.is_none());
        assert!(approved.capabilities.endpoints.as_map().is_empty());
    }

    /// The inverse pin (U4): `sql:<connection>` now gates something on a run.
    /// The run's decider is the frozen session policy seeded from `--allow`
    /// (`assembly.rs`), so the stated token rides the approval verbatim and
    /// the journal — which pre-answers nothing without it — carries it to a
    /// resume. The scope builds no run capability: it is a per-call grant
    /// word, seeded, not a plan capability. A run started with
    /// `--allow sql:analytics` behaves differently from one without it — the
    /// decider-level proof is
    /// `a_run_started_with_allow_sql_gates_the_sql_family_s_asks`
    /// (`prompt_approval_tests`).
    #[test]
    fn sql_gates_a_run_s_sql_family_asks() {
        let Ok(approved) = parse(&["sql:analytics".to_string()], Surface::Run) else {
            panic!("`sql:analytics` gates the run's SQL-family asks and must approve");
        };
        assert!(
            approved.tokens.iter().any(|token| token == "sql:analytics"),
            "the run's approval carries the token verbatim — the frozen decider's \
             seed is the word the user typed: {:?}",
            approved.tokens
        );
        // The scope is a grant word, not a capability: nothing else moved.
        assert!(!approved.capabilities.workspace_write);
        assert!(!approved.capabilities.scratch);
        assert!(approved.capabilities.fetch.is_none());
        assert!(approved.capabilities.runner.is_none());
        assert!(approved.capabilities.interpreter.is_none());
        assert!(approved.capabilities.endpoints.as_map().is_empty());
    }

    /// `endpoint:` is refused on the session surface too — a session binds
    /// no per-step endpoint roles — and with its own reason: the run's
    /// words ("every episode calls the orchestrator endpoint") describe a
    /// run, not a session, so the session refusal is pinned to its own
    /// phrase and must not borrow the run's. The run surface's refusal
    /// (with the run's reason) is pinned by
    /// `a_scope_nothing_consumes_is_refused_rather_than_silently_approved`.
    #[test]
    fn endpoint_is_refused_on_the_session_surface_with_its_own_reason() {
        let Err(error) = parse(&["endpoint:analyst=fast".to_string()], Surface::Session) else {
            panic!("a session binds no per-step endpoint roles, so `/allow endpoint:` refuses");
        };
        assert!(
            error.contains("not available yet"),
            "the refusal must say why, got: {error}"
        );
        assert!(
            error.contains("binds no per-step endpoint roles"),
            "the session refusal must state its own reason: {error}"
        );
        assert!(
            !error.contains("every episode calls the orchestrator endpoint"),
            "the session refusal must not borrow the run's reason: {error}"
        );
        assert!(
            error.contains("endpoint:analyst=fast"),
            "the refusal must name the scope, got: {error}"
        );
    }

    /// The `sql:` payload is a connection's registry name, judged by the
    /// same name-shape rule `endpoint:` payloads are: non-empty, bounded,
    /// no control characters, no whitespace. A payload that fails the shape
    /// is a typed usage error on every surface that accepts the family —
    /// the shape rule is the grammar's, shared by both surfaces.
    #[test]
    fn the_sql_payload_is_judged_by_the_name_shape_rule() {
        assert!(
            parse(&["sql:".to_string()], Surface::Session).is_err(),
            "empty payload"
        );
        assert!(
            parse(&["sql:prod eu".to_string()], Surface::Session).is_err(),
            "whitespace is never a connection name"
        );
        assert!(
            parse(&["sql:has\nnewline".to_string()], Surface::Session).is_err(),
            "control characters are never a connection name"
        );
        assert!(
            parse(&["sql:prod eu".to_string()], Surface::Run).is_err(),
            "the shape rule is the grammar's, not a surface's"
        );
        let Ok(approved) = parse(&["sql:Analytics_2".to_string()], Surface::Session) else {
            panic!("a name-shaped payload parses");
        };
        assert!(
            approved.tokens.contains(&"sql:Analytics_2".to_string()),
            "the payload rides the token verbatim — /grants shows the word the \
             user typed, got: {:?}",
            approved.tokens
        );
    }

    /// A fetch token is normalised at parse — the URL parser's spelling —
    /// on both surfaces: the approval carries `fetch:https+example.com`
    /// whatever casing the user typed, the parsed destination is the
    /// lowercased one, and an already-lowercase token is today's bytes
    /// unchanged. The run engine's fetch policy compares the URL parser's
    /// lowercased form, and the grant token suggester produces it from the
    /// call's URL, so the token the approval carries must be that same
    /// string: a seeded token and a suggested token are one string for one
    /// destination (U6 defect 2 — the verbatim seed never matched the
    /// lowercased suggestion, so `/grants` listed a grant that pre-answered
    /// nothing).
    #[test]
    fn a_fetch_token_is_normalised_at_parse_on_both_surfaces() {
        for surface in [Surface::Run, Surface::Session] {
            let Ok(approved) = parse(&["fetch:HTTPS+Example.com".to_string()], surface) else {
                panic!("a mixed-case fetch token parses on {surface:?}");
            };
            assert_eq!(
                approved.tokens,
                vec!["fetch:https+example.com".to_owned()],
                "the approval carries the URL parser's spelling on {surface:?}"
            );
            let fetch = approved
                .capabilities
                .fetch
                .as_ref()
                .expect("the fetch scope is approved");
            assert_eq!(
                fetch.destinations,
                vec![
                    Destination::new("https", "example.com")
                        .expect("the normalised destination is shape-valid")
                ],
                "the parsed destination is the normalised one on {surface:?}"
            );
        }
        let approved = parse(&["fetch:https+example.com".to_string()], Surface::Run)
            .expect("a lowercase fetch token parses");
        assert_eq!(
            approved.tokens,
            vec!["fetch:https+example.com".to_owned()],
            "an already-lowercase token keeps its exact bytes"
        );
    }

    /// The refusal's tail names the surface's own next step. A run re-runs
    /// `saya run` without the scope — those exact bytes are pinned. A slash
    /// command has nothing to re-run: `/allow` is re-issued without the
    /// token, so the session tail must not tell the user to re-run anything.
    #[test]
    fn the_refusal_s_tail_names_the_surface_s_own_next_step() {
        let Err(run_error) = parse(&["endpoint:analyst=fast".to_string()], Surface::Run) else {
            panic!("`endpoint:` is refused on the run surface");
        };
        assert!(
            run_error.ends_with("Re-run without it."),
            "the run surface's tail keeps its exact bytes: {run_error}"
        );
        let Err(session_error) = parse(&["endpoint:analyst=fast".to_string()], Surface::Session)
        else {
            panic!("`endpoint:` is refused on the session surface");
        };
        assert!(
            session_error.ends_with("Re-issue /allow without it."),
            "a slash command is re-issued, not re-run: {session_error}"
        );
        assert!(
            !session_error.contains("Re-run"),
            "nothing at a slash command is a re-run: {session_error}"
        );
    }

    /// The empty approval is stateable on the session surface too: `/allow
    /// none` parses and carries nothing grantable — the session command
    /// layer reads the `none` from the stated tokens and seeds nothing.
    #[test]
    fn none_states_the_empty_approval_on_the_session_surface_too() {
        let Ok(approved) = parse(&["none".to_string()], Surface::Session) else {
            panic!("`none` must state the empty approval on the session surface");
        };
        assert_eq!(approved.tokens, vec!["none".to_string()]);
        assert!(!approved.capabilities.workspace_write);
        assert!(!approved.capabilities.scratch);
        assert!(approved.capabilities.fetch.is_none());
        assert!(approved.capabilities.runner.is_none());
        assert!(approved.capabilities.interpreter.is_none());
        assert!(approved.capabilities.endpoints.as_map().is_empty());
    }

    /// The scope that *is* wired keeps working — the fix must refuse the
    /// inert ones without breaking the capability a run can actually use.
    #[test]
    fn workspace_write_is_wired_and_still_approves() {
        let Ok(approved) = parse(&["workspace-write".to_string()], Surface::Run) else {
            panic!("the wired scope must still approve");
        };
        assert!(approved.capabilities.workspace_write);
        assert!(!approved.capabilities.scratch);
        assert!(approved.capabilities.fetch.is_none());
        assert!(approved.capabilities.runner.is_none());
        assert!(approved.capabilities.interpreter.is_none());
        assert!(approved.capabilities.endpoints.as_map().is_empty());
    }

    /// The inverse pin's first half: `scratch` is wired, so `--allow scratch`
    /// approves the scope — the deletion of its refusal entry alone proves
    /// nothing, this does. The second half (its tool in the universe of the
    /// steps that asked for it) lives beside the toolset builder's tests.
    #[test]
    fn scratch_is_wired_and_still_approves() {
        let Ok(approved) = parse(&["scratch".to_string()], Surface::Run) else {
            panic!("the wired scope must approve");
        };
        assert!(approved.capabilities.scratch);
        assert!(!approved.capabilities.workspace_write);
        assert!(approved.capabilities.fetch.is_none());
        assert!(approved.capabilities.runner.is_none());
        assert!(approved.capabilities.interpreter.is_none());
        assert!(approved.capabilities.endpoints.as_map().is_empty());
    }

    /// The inverse pin's first half: `runner:` is wired, so `--allow
    /// runner:bench` approves the scope — the deletion of its refusal entry
    /// alone proves nothing, this does. The second half (its tool in the
    /// universe of the steps that asked, and the narrowed-allowlist gate
    /// that rides the toolset builder) lives beside the toolset builder's
    /// tests.
    #[test]
    fn runner_is_wired_and_still_approves() {
        let Ok(approved) = parse(&["runner:bench".to_string()], Surface::Run) else {
            panic!("the wired scope must approve");
        };
        let runner = approved
            .capabilities
            .runner
            .expect("the runner scope must be approved");
        assert_eq!(runner.programs, vec!["bench".to_owned()]);
        assert!(!approved.capabilities.workspace_write);
        assert!(!approved.capabilities.scratch);
        assert!(approved.capabilities.fetch.is_none());
        assert!(approved.capabilities.endpoints.as_map().is_empty());
    }

    /// The empty approval is stateable: `--allow none` starts a run that
    /// approves nothing at all. One of the grammar's five capability
    /// scopes is refused, so without this token the only way to start any
    /// run — including a purely read-only one — would be approving one of
    /// the wired scopes, which would turn a scope that means something into
    /// boilerplate everyone types.
    #[test]
    fn none_states_the_empty_approval_for_a_read_only_run() {
        let Ok(approved) = parse(&["none".to_string()], Surface::Run) else {
            panic!("`none` must state the empty approval");
        };
        assert!(!approved.capabilities.workspace_write);
        assert!(!approved.capabilities.scratch);
        assert!(approved.capabilities.fetch.is_none());
        assert!(approved.capabilities.runner.is_none());
        assert!(approved.capabilities.interpreter.is_none());
        assert!(approved.capabilities.endpoints.as_map().is_empty());
    }

    /// `none` names "nothing", so beside a scope it would state both
    /// nothing and something — refused, not resolved in the user's favour.
    #[test]
    fn none_must_be_the_only_token_when_stated() {
        let Err(error) = parse(
            &["none".to_string(), "workspace-write".to_string()],
            Surface::Run,
        ) else {
            panic!("`none` beside a scope must refuse");
        };
        assert!(
            error.contains("`none` states the empty scope set"),
            "the refusal must name the contradiction: {error}"
        );
        assert!(error.contains("known scopes:"), "got: {error}");
    }

    /// An unknown token stays a usage error, and its message still lists the
    /// grammar — the refusal for "not yet" must not swallow the refusal for
    /// "no such thing".
    #[test]
    fn an_unknown_scope_is_still_a_usage_error_naming_the_grammar() {
        let Err(error) = parse(&["wat".to_string()], Surface::Run) else {
            panic!("unknown scope must refuse");
        };
        assert!(error.contains("unknown scope `wat`"), "got: {error}");
        assert!(error.contains("known scopes:"), "got: {error}");
    }

    /// The grammar mirror, both directions (the interpreter approval's
    /// design §1): a name the runner refuses rides the interpreter family
    /// only, and a name the runner can run rides the runner family only.
    /// One list, two doors — `--allow runner:python3` names the fix
    /// (`interpreter:`), `--allow interpreter:ripgrep` names its fix
    /// (`runner:`), and neither survives to become a scope the runner
    /// refuses at call time.
    #[test]
    fn a_refused_name_is_the_interpreter_family_s_not_the_runner_s() {
        let Err(error) = parse(&["runner:python3".to_string()], Surface::Run) else {
            panic!("`runner:python3` must be a typed usage error");
        };
        assert!(
            error.contains("`python3` is a shell or interpreter the runner refuses"),
            "the refusal must say why: {error}"
        );
        assert!(
            error.contains("`interpreter:python3`"),
            "the refusal must name the right family: {error}"
        );

        let Err(error) = parse(&["interpreter:ripgrep".to_string()], Surface::Run) else {
            panic!("`interpreter:ripgrep` must be a typed usage error");
        };
        assert!(
            error.contains("`ripgrep` is not a shell or interpreter the runner refuses"),
            "the refusal must say why: {error}"
        );
        assert!(
            error.contains("`runner:ripgrep`"),
            "the refusal must name the right family: {error}"
        );
    }

    /// The interpreter family approves its own token: `--allow
    /// interpreter:python3` grants exactly that interpreter, and the other
    /// spellings stay refused — a shell the family's list carries is a
    /// member, so `interpreter:bash` parses (the family is the refusal list
    /// verbatim; the design does not pretend bash is unreachable by
    /// refusing to spell it), while a program the runner can run is refused
    /// here.
    #[test]
    fn the_interpreter_family_approves_its_own_tokens() {
        let Ok(approved) = parse(&["interpreter:python3".to_string()], Surface::Run) else {
            panic!("`interpreter:python3` must approve");
        };
        let interpreters = approved
            .capabilities
            .interpreter
            .expect("the interpreter scope must be approved");
        assert_eq!(interpreters.programs, vec!["python3".to_owned()]);
        assert!(approved.capabilities.runner.is_none());
        assert!(!approved.capabilities.workspace_write);
        assert!(!approved.capabilities.scratch);
        assert!(approved.capabilities.fetch.is_none());

        let Ok(approved) = parse(&["interpreter:bash".to_string()], Surface::Run) else {
            panic!("`interpreter:bash` parses — the family is the refusal list");
        };
        let interpreters = approved
            .capabilities
            .interpreter
            .expect("the interpreter scope must be approved");
        assert_eq!(interpreters.programs, vec!["bash".to_owned()]);
    }

    /// The three help surfaces that enumerate the scope grammar — the clap
    /// `--allow` doc comment, the `/run` slash help, and the `/allow` slash
    /// help — and [`NOT_YET_WIRED`] must agree, surface by surface, in both
    /// directions: a family the help names as refused on that surface must
    /// sit in that surface's refusal list, and every refusal-list entry must
    /// be named as refused in that surface's help. The surfaces are
    /// *supposed* to disagree — `/allow` (a session) accepts `sql:` where a
    /// run refuses it, and refuses `endpoint:` with its own reason — so the
    /// expected refusal set is read per surface, and an accidental
    /// disagreement is caught exactly where it lands: the run surfaces
    /// claiming `sql:` accepted (a scope a run's decider consults nothing
    /// for), or `/allow`'s help claiming `sql:` refused (a grant the session
    /// can state), each fails this on its own surface while the other
    /// surfaces stay green. Each S-slice deleted its entry here and updated
    /// `docs/commands.md` while a help surface kept claiming every remaining
    /// family was refused — a help surface denying a capability the surface
    /// has, the exact class this test turns red the day the lists diverge
    /// again, either direction, on either surface. The families come from
    /// [`KNOWN`] itself, so a scope added to the grammar without touching
    /// every surface is caught too. "Refused" is read per clause (a
    /// `.`-sentence cut at `;` and parentheses — see [`claims_refused`]):
    /// a wired family must never share its own clause with a refusal word,
    /// and a refused family must. The clause cut exists because the `/allow`
    /// help lists the wired scopes in one sentence whose `command:`
    /// parenthetical carries that family's own lane gate; a sentence-wide
    /// reader attributes that gate to every listed scope.
    ///
    /// `command:` is deliberately outside this agreement: it is refused on
    /// runs by permanent policy (`run_command_refusal`, its own class — the
    /// wiring-shaped list never carries it) and parsed on sessions subject
    /// to the composition gate, so no `NOT_YET_WIRED` entry can state its
    /// status. Its help wording is pinned by
    /// `a_run_refuses_command_scopes_by_design` (run surfaces) and the
    /// `/allow` help names the lane gate instead.
    #[test]
    fn the_help_surfaces_and_the_refusal_list_agree() {
        let surfaces = [
            ("the clap `--allow` help", clap_allow_help(), Surface::Run),
            (
                "the `/run` slash help",
                crate::slash::command_help("run")
                    .expect("/run has per-command help")
                    .to_string(),
                Surface::Run,
            ),
            (
                "the `/allow` slash help",
                crate::slash::command_help("allow")
                    .expect("/allow has per-command help")
                    .to_string(),
                Surface::Session,
            ),
        ];
        for (surface_name, text, surface) in &surfaces {
            for (family, token) in grammar_tokens() {
                assert!(
                    text.contains(token),
                    "{surface_name} must name the scope `{token}` — a surface that \
                     omits a scope leaves its status to the reader's guess, got: {text}"
                );
                // `command:` is outside the wiring-shaped agreement (see the
                // doc comment): its run refusal is permanent policy and its
                // session status is a composition gate, not a list entry.
                if family == "command" {
                    continue;
                }
                let claimed = claims_refused(text, token);
                let listed = NOT_YET_WIRED
                    .iter()
                    .any(|entry| entry.family == family && entry.surface == *surface);
                assert_eq!(
                    claimed,
                    listed,
                    "{surface_name} and NOT_YET_WIRED disagree about `{family}` on \
                     {surface:?}: the help {} while the refusal list {}. A scope the \
                     next slice wires must stop being refused in that surface's help; \
                     one still unwired there must stay refused.",
                    if claimed {
                        "claims it is refused"
                    } else {
                        "does not claim it is refused"
                    },
                    if listed {
                        "still refuses it"
                    } else {
                        "no longer refuses it"
                    },
                );
            }
        }
    }

    /// The `--allow` help text exactly as clap derives it from the doc
    /// comment — the string a user reads in `saya run --help`.
    fn clap_allow_help() -> String {
        use clap::CommandFactory as _;
        let mut cmd = crate::cli::Cli::command();
        let run = cmd
            .find_subcommand_mut("run")
            .expect("`saya run` is declared");
        let allow = run
            .get_arguments()
            .find(|arg| arg.get_id() == "allow")
            .expect("`--allow` is declared on `saya run`");
        allow
            .get_long_help()
            .or_else(|| allow.get_help())
            .expect("the --allow doc comment reaches clap")
            .to_string()
    }

    /// The grammar's families, derived from [`KNOWN`] itself: each token
    /// paired with its family key (the prefix before `:`, the name
    /// [`NOT_YET_WIRED`] entries use). A token added to [`KNOWN`] without a
    /// surface update fails the parity test, not just the parse.
    fn grammar_tokens() -> Vec<(&'static str, &'static str)> {
        let list = KNOWN
            .split_once("known scopes: ")
            .expect("`KNOWN` must name the scopes it refuses around")
            .1;
        list.split(", ")
            .map(|token| {
                let family = token.split(':').next().unwrap_or(token);
                (family, token)
            })
            .collect()
    }

    /// True when some clause of the surface pairs the scope token with a
    /// refusal word — the shape a refusal claim takes in these surfaces. A
    /// clause is a `.`-sentence cut further at `;`, `:` (outside the token's
    /// own shape), and `(`/`)`, so a refusal scoped to one family inside a
    /// parenthetical (the `/allow` help's `command:` lane gate) does not leak
    /// onto every token the outer sentence lists ("the wired scopes are …").
    /// A real disagreement still reads: a token sharing its own clause with a
    /// refusal word stays red in both directions.
    fn claims_refused(surface: &str, token: &str) -> bool {
        surface.split('.').any(|sentence| {
            sentence.contains(token)
                && sentence
                    .split([';', '(', ')'])
                    .any(|clause| clause.contains(token) && clause.contains("refus"))
        })
    }

    /// The reader above must stay able to tell a scoped refusal from list
    /// prose: a refusal word that shares the token's own parenthetical is a
    /// claim about that token, while one cut off by the same boundaries —
    /// the `/allow` lane gate living one clause away from the wired list — is
    /// not. A reader that could not make that distinction either cries wolf
    /// on every listed scope or goes blind to a real disagreement; both
    /// halves are pinned here so a future edit cannot weaken one into the
    /// other.
    #[test]
    fn the_refusal_reader_attributes_a_refusal_to_its_own_clause() {
        let allow = crate::slash::command_help("allow").expect("/allow has per-command help");
        assert!(allow.contains("workspace-write"));
        assert!(
            !claims_refused(allow, "workspace-write"),
            "the /allow lane gate sits in command:'s own clause, not workspace-write's: {allow}"
        );
        assert!(
            claims_refused(allow, "endpoint:<role>=<endpoint>"),
            "endpoint:'s own clause does refuse it, and the reader must say so: {allow}"
        );
        let scoped = "the wired scopes are `workspace-write`, and `sql:<connection>`. \
            `endpoint:<role>=<endpoint>` is refused here: nothing binds it.";
        assert!(
            claims_refused(scoped, "endpoint:<role>=<endpoint>"),
            "a real refusal claim must still read as one: {scoped}"
        );
        assert!(
            !claims_refused(scoped, "workspace-write"),
            "a listed scope one sentence away must not read as refused: {scoped}"
        );
    }

    /// H1 red: the `command:<program>` family joins the grammar on the
    /// session surface — payload a bare name, shells included, no family
    /// mirror. Written before the family exists, so the token is refused as
    /// unknown today (the failure is the grammar, not the composition seam;
    /// the composed-vs-bare half is pinned at the `/allow` level by
    /// `command_tokens_parse_on_a_composed_session_and_refuse_on_a_bare_one`).
    #[test]
    fn command_joins_the_grammar_on_the_session_surface() {
        let Ok(approved) = parse(&["command:npm".to_string()], Surface::Session) else {
            panic!("`command:npm` must parse on the session surface once H1 lands");
        };
        assert!(
            approved.tokens.iter().any(|token| token == "command:npm"),
            "the approval carries the token verbatim: {:?}",
            approved.tokens
        );
        // The family mirror deliberately does not apply here: shells ride
        // `command:`, never `interpreter:`-by-mirror.
        let Ok(shell) = parse(&["command:bash".to_string()], Surface::Session) else {
            panic!("`command:bash` parses — shells are included, no mirror");
        };
        assert!(
            shell.tokens.iter().any(|token| token == "command:bash"),
            "the shell token rides verbatim: {:?}",
            shell.tokens
        );
    }

    /// H1 red: the `command:` payload is a bare name — never a path or a
    /// traversal. Written before the family exists, so the refusal today is
    /// the unknown-scope error rather than the shape rule.
    #[test]
    fn the_command_payload_is_a_bare_name_never_a_path() {
        for token in ["command:", "command:bin/npm", "command:../npm"] {
            let Err(error) = parse(&[token.to_string()], Surface::Session) else {
                panic!("`{token}` must refuse: the payload is a bare name");
            };
            assert!(
                error.contains("bare name"),
                "the refusal states the shape rule: {error}"
            );
        }
    }

    /// H1 red: on the run surface `command:` is a permanent policy refusal in
    /// its own class — never an absence claim, with its own pinned wording.
    /// Written before the refusal exists, so the run parser's unknown-scope
    /// error is what fires today.
    #[test]
    fn a_run_refuses_command_scopes_by_design() {
        for token in ["command:npm", "command:bash"] {
            let Err(error) = parse(&[token.to_string()], Surface::Run) else {
                panic!("a run never gets this lane");
            };
            assert!(
                error.contains("not available on runs, by design"),
                "the run refusal is policy, pinned: {error}"
            );
            assert!(
                error.contains(super::RUN_COMMAND_REFUSAL_PIN),
                "the pin rides the production constant: {error}"
            );
            assert!(
                error.contains("unconfined host execution"),
                "the refusal says what the scope names: {error}"
            );
            assert!(
                error.contains("runner:") && error.contains("interpreter:"),
                "the refusal steers to a sandboxed scope: {error}"
            );
            assert!(
                !error.contains("not available yet"),
                "never an absence claim — this refusal never leaves: {error}"
            );
            assert!(
                error.contains(token),
                "the refusal names the scope: {error}"
            );
            assert!(error.contains("Re-run without it."), "got: {error}");
        }
    }

    /// The permanent refusal is its own class: the wiring-shaped list never
    /// carries the `command` family on any surface — the run refusal above is
    /// policy, and the session surface parses the family rather than
    /// refusing it.
    #[test]
    fn the_command_family_is_never_a_wiring_refusal() {
        for entry in NOT_YET_WIRED {
            assert!(
                entry.family != "command",
                "`command` must not join the wiring-shaped list on any surface"
            );
        }
    }
}
