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
