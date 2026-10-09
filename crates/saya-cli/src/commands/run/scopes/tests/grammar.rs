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
