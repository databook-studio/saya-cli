/// A held `shell:<prefix>` grant advertises the session's `shell` exactly
/// where a granted call can run — the promise the advertisement makes. The
/// two sides are read independently: the advertisement from the session's own
/// definitions gate, the decision from the composed shell that gate speaks
/// for. An operator that answers `yes` stands in for a surface that can obtain
/// an approval, so "admitted or askable" is one answer; where neither an
/// operator nor a sandbox is there, only the store can pre-answer a call, so
/// only a held grant makes the shell usable.
#[tokio::test]
async fn the_shell_grant_advertises_the_shell_exactly_where_a_granted_call_runs() {
    let project = worktree("shell-grant-advertise");
    let state = temp_dir("shell-grant-advertise-state");
    let read_only_proves = read_only_sandbox_proves(&project);
    // The matrix is read twice: over this host as it is, and over the same
    // session with its writable sandbox settled at "does not prove" — a
    // headless asking turn with no lane of its own, where a held grant is the
    // one thing that can make the call run, so the grant rows decide there.
    for (without_ask_proof, ask_proves) in [(false, ask_sandbox_proves(&project)), (true, false)] {
        let universe = compose(&session_runtime(), &project, &state);
        let universe = if without_ask_proof {
            universe.without_ask_proof()
        } else {
            universe
        };
        let granted_call = serde_json::json!({ "command": "cargo test" });
        let definition = crate::agent::tools::shell_policy::definition();
        for (mode, can_obtain_approval, held, expected) in [
            (ApprovalPolicy::Ask, true, false, true),
            (ApprovalPolicy::Ask, true, true, true),
            (ApprovalPolicy::Ask, false, false, ask_proves),
            (ApprovalPolicy::Ask, false, true, true),
            (ApprovalPolicy::AcceptEdits, true, false, true),
            (ApprovalPolicy::AcceptEdits, false, false, ask_proves),
            (ApprovalPolicy::AcceptEdits, false, true, true),
            (ApprovalPolicy::Bypass, false, false, true),
            (ApprovalPolicy::Bypass, false, true, true),
            (ApprovalPolicy::ReadOnly, true, false, read_only_proves),
            (ApprovalPolicy::ReadOnly, true, true, read_only_proves),
            (ApprovalPolicy::Never, true, false, false),
            (ApprovalPolicy::Never, true, true, false),
        ] {
            let grants = saya_agent::SessionGrants::default();
            if held {
                grants.grant("shell:cargo test");
            }
            universe.bind_grants(&grants);
            let names = advertised(&universe, AgentMode::Build, mode, can_obtain_approval);
            let advertised_here = names.contains(&"shell".to_string());
            let surface: Option<Arc<dyn ApprovalDecider>> = can_obtain_approval
                .then(|| RecordingOperator::new(true) as Arc<dyn ApprovalDecider>);
            let runs = match universe.shell(mode, surface, CancellationToken::new()) {
                Some(shell) => shell.approve(&definition, &granted_call).await,
                None => false,
            };
            let facts = format!(
                "{mode:?} (can_obtain_approval={can_obtain_approval}, grant held={held}, \
                 ask sandbox proves={ask_proves})"
            );
            assert_eq!(
                advertised_here, expected,
                "under {facts} the shell is advertised exactly where a granted call can run: \
                 {names:?}"
            );
            assert_eq!(
                advertised_here, runs,
                "under {facts} the advertisement and the decision must agree: {names:?}"
            );
        }
    }
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// The session's shell reads the session's store: the call the store holds is
/// pre-answered, so a refusing approval surface is never put the question —
/// the job's "yes for this run", on the surface that keeps it per turn rather
/// than on an operator. A call the store does not name still asks, and the
/// surface's refusal is its answer.
#[tokio::test]
async fn a_shell_grant_pre_answers_the_sessions_own_ask() {
    let project = worktree("shell-grant-headless");
    let state = temp_dir("shell-grant-headless-state");
    let universe = compose(&session_runtime(), &project, &state);
    let grants = saya_agent::SessionGrants::default();
    grants.grant("shell:cargo test");
    universe.bind_grants(&grants);
    let definition = crate::agent::tools::shell_policy::definition();
    let surface_operator = RecordingOperator::new(false);
    let surface: Arc<dyn ApprovalDecider> = surface_operator.clone();
    let shell = universe
        .shell(ApprovalPolicy::Ask, Some(surface), CancellationToken::new())
        .expect("a host lane composes wherever a root binds");
    assert!(
        shell
            .approve(&definition, &serde_json::json!({ "command": "cargo test" }))
            .await,
        "the granted call is admitted although the surface refuses everything"
    );
    assert_eq!(
        surface_operator.calls(),
        0,
        "a granted call never reaches the surface"
    );
    assert!(
        !shell
            .approve(&definition, &serde_json::json!({ "command": "cargo build" }))
            .await,
        "a call outside the named prefix keeps the surface's answer"
    );
    assert_eq!(surface_operator.calls(), 1, "`cargo build` still asks");
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// A posture that never consults grants stays as it was: `read-only` shows
/// the shell exactly where its sandbox proves — the grant changes nothing
/// there — `never` shows none, and Plan hides it with the rest of the
/// write-shaped surface. A dormant grant under a restrictive posture is
/// dormant.
#[test]
fn a_shell_grant_widens_no_posture_that_never_consults_grants() {
    let project = worktree("shell-grant-postures");
    let state = temp_dir("shell-grant-postures-state");
    let universe = compose(&session_runtime(), &project, &state);
    let without = {
        universe.bind_grants(&saya_agent::SessionGrants::default());
        (
            advertised(&universe, AgentMode::Build, ApprovalPolicy::ReadOnly, true),
            advertised(&universe, AgentMode::Build, ApprovalPolicy::Never, true),
            advertised(&universe, AgentMode::Plan, ApprovalPolicy::Ask, true),
        )
    };
    let with = {
        let grants = saya_agent::SessionGrants::default();
        grants.grant("shell:cargo test");
        universe.bind_grants(&grants);
        (
            advertised(&universe, AgentMode::Build, ApprovalPolicy::ReadOnly, true),
            advertised(&universe, AgentMode::Build, ApprovalPolicy::Never, true),
            advertised(&universe, AgentMode::Plan, ApprovalPolicy::Ask, true),
        )
    };
    assert_eq!(
        with.0, without.0,
        "`read-only` never consults grants: the sandbox lane alone decides"
    );
    assert!(
        !with.1.contains(&"shell".to_string()),
        "`never` keeps no lane, grant or not: {:?}",
        with.1
    );
    assert!(
        !with.2.contains(&"shell".to_string()),
        "Plan denies before the approval match, grant or not: {:?}",
        with.2
    );
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// `read-only` never consults a grant: where the write-denied sandbox does
/// not prove, the mode has no shell lane at all — no shell to compose, none
/// to run a call, none to advertise — and a held `shell:` grant must not
/// conjure one. The shape is Linux's (its allow-only Landlock refuses the
/// write denial), stated here over the read-only proof so it can be read on a
/// host where that proof succeeds.
#[test]
fn a_shell_grant_never_advertises_the_shell_under_read_only() {
    let project = worktree("shell-grant-read-only");
    let state = temp_dir("shell-grant-read-only-state");
    let universe = compose(&session_runtime(), &project, &state).without_read_only_proof();
    let grants = saya_agent::SessionGrants::default();
    grants.grant("shell:cargo test");
    universe.bind_grants(&grants);
    assert!(
        universe
            .shell(ApprovalPolicy::ReadOnly, None, CancellationToken::new())
            .is_none(),
        "with no write-denied sandbox, read-only composes no shell at all"
    );
    let names = advertised(&universe, AgentMode::Build, ApprovalPolicy::ReadOnly, true);
    assert!(
        !names.contains(&"shell".to_string()),
        "a `shell:` grant cannot advertise a shell read-only never composed: {names:?}"
    );
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// No workspace root, no shell: the session's sandboxes are proven over a
/// bound root, and the operating system's own temporary directory is the
/// root the bind refuses. A grant names a lane nothing composed.
#[test]
fn a_shell_grant_names_nothing_without_a_workspace_root() {
    let plain = std::env::temp_dir();
    let state = temp_dir("shell-grant-no-root-state");
    let universe = compose(&session_runtime(), &plain, &state);
    assert!(
        universe.root().is_none(),
        "the bare temporary directory binds no root"
    );
    let grants = saya_agent::SessionGrants::default();
    grants.grant("shell:cargo test");
    universe.bind_grants(&grants);
    for mode in [
        ApprovalPolicy::Ask,
        ApprovalPolicy::AcceptEdits,
        ApprovalPolicy::Bypass,
        ApprovalPolicy::ReadOnly,
    ] {
        let names = advertised(&universe, AgentMode::Build, mode, true);
        assert!(
            !names.contains(&"shell".to_string()),
            "no root binds, so no shell is composed under {mode:?}: {names:?}"
        );
    }
    let _ = fs::remove_dir_all(&state);
}
