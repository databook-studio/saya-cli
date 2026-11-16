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

/// Both surfaces refuse the removed runner and interpreter families, each
/// in its own words: a run names the plan's `shell` tool, a session names
/// the grant families it does not carry. The tokens still parse — a
/// persisted plan or a held grant naming them is not an unknown scope —
/// and neither builds a capability on either surface.
#[test]
fn the_removed_runner_and_interpreter_families_are_refused_on_both_surfaces() {
    for token in ["runner:report", "runner:python3", "interpreter:python3"] {
        let Err(error) = parse(&[token.to_string()], Surface::Session) else {
            panic!("`{token}` names a removed family and must be refused");
        };
        assert!(
            error.contains(REMOVED_RUNNER_GRANT_REFUSAL),
            "the session refusal states the removal: {error}"
        );
        assert!(
            error.contains(token),
            "the refusal names the scope: {error}"
        );
    }
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
/// disagreement is caught exactly where it lands: the run surface
/// claiming `sql:` accepted (a scope a run's decider consults nothing
/// for), or `/allow`'s help claiming `sql:` refused (a grant the session
/// can state), each fails this on its own surface while the other
/// surfaces stay green. A help surface denying a capability the surface
/// has is the exact class this test turns red the day the lists diverge,
/// in either direction, on either surface. The families come from
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
