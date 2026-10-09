/// A launch allowing `command:x` seeds the token: the seed seeds the grant
/// (it no longer implies composition — the root composes the lane). The
/// bare composition still refuses — pinned by
/// `command_tokens_parse_on_a_composed_session_and_refuse_on_a_bare_one`.
/// Kept whole (reason: only the implies-composition clause moved; the seed
/// behaviour stays).
#[test]
fn a_launch_allowing_command_x_composes_the_lane_and_seeds_the_token() {
    use crate::approval_facts::{ApprovalFacts, HostFacts};
    let grants = saya_agent::SessionGrants::default();
    let composed = ApprovalFacts {
        host: Some(HostFacts::for_tests()),
        ..ApprovalFacts::default()
    };
    let seeded = crate::interactive::session::permissions::grants::seed_launch_allow(
        &["command:npm".to_owned()],
        &composed,
        &grants,
    );
    let seeded = seeded.expect("a launch seed parses on the session surface");
    assert!(
        seeded.iter().any(|token| token == "command:npm"),
        "the launch seeds the token: {seeded:?}"
    );
    assert!(
        grants.is_granted("command:npm"),
        "the seed token is in force from launch"
    );
}

/// `command:` tokens parse on a composed session and refuse on a bare one —
/// with the lane off, `/allow command:<x>` refuses with the launch wording,
/// in the surface-aware refusal register.
#[test]
fn command_tokens_parse_on_a_composed_session_and_refuse_on_a_bare_one() {
    use crate::approval_facts::{ApprovalFacts, HostFacts};
    let composed = ApprovalFacts {
        host: Some(HostFacts::for_tests()),
        ..ApprovalFacts::default()
    };
    let dir = state_dir("command-composed");
    let grants = saya_agent::SessionGrants::default();
    let journal = SessionJournal::open(&dir);
    let message = allow(&["command:npm".to_owned()], &composed, &grants, &journal)
        .expect("a composed session parses command: tokens");
    assert!(
        message.contains("command:npm"),
        "the composed session seeds the token: {message}"
    );
    assert!(grants.is_granted("command:npm"));

    let bare = ApprovalFacts::default();
    let grants = saya_agent::SessionGrants::default();
    let journal = SessionJournal::open(state_dir("command-bare"));
    let error = match allow(&["command:npm".to_owned()], &bare, &grants, &journal) {
        Err(error) => error,
        Ok(message) => panic!("a bare session must refuse `command:npm`: {message}"),
    };
    assert!(
        error.contains("gates nothing in this session")
            && error.contains("no workspace root is bound"),
        "the bare-session refusal names the workspace-root fact: {error}"
    );
    assert!(
        !error.contains("--host-commands"),
        "no deleted flag in the refusal: {error}"
    );
}

/// A seed where the store holds only tokens a run refuses: nothing is
/// forwarded and every token is named — the child's `--allow` is untouched.
#[test]
fn a_seed_request_names_every_token_it_does_not_forward() {
    use crate::interactive::session::permissions::grants::{run_seed, seed_message};
    let tokens = ["endpoint:analyst=fast".to_owned()];
    let seed = run_seed(&tokens);
    assert!(seed.forwarded.is_empty(), "a run accepts none of it");
    let message = seed_message(&seed);
    assert!(
        message.contains("not forwarded") && message.contains("endpoint:analyst=fast"),
        "the dropped tokens are named: {message}"
    );
    assert!(
        !message.contains("seeded"),
        "nothing was forwarded, so the message claims no seeding: {message}"
    );
}
