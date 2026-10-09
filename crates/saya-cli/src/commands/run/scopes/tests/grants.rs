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
    let Err(session_error) = parse(&["endpoint:analyst=fast".to_string()], Surface::Session) else {
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
