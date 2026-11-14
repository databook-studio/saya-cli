/// The contrast the session journal exists to make: a resumed session
/// inherits no grant from the journal. The journal is the audit record of
/// what the user consented to — never a grant source, the deliberate
/// opposite of runs, whose grants ARE re-derived from their journal. The
/// resume re-uses the same session id and state dir; the policy it builds
/// there is empty by construction, and the journal is left byte-identical.
#[test]
fn a_resumed_session_inherits_no_grant_from_the_journal() {
    let root = temp_dir("resume-journal-root");
    let id = "resume-journal-1";
    {
        let first = crate::interactive::session::runtime::SessionRuntime::acquire(
            &session_runtime(),
            None,
            true,
            None,
            id,
            ApprovalPolicy::Ask,
            &root,
        )
        .expect("the first process acquires");
        // A previous process granted a token — a `[s]` answer, journalled.
        first
            .journal()
            .granted("sql:analytics", saya_store::GrantSource::Prompt)
            .expect("the first process journals its grant");
        drop(first);
    }
    let before = saya_store::SessionJournal::open(root.join(id))
        .read()
        .expect("the journal reads");
    // The resume re-enters the same state dir under the same id...
    let resumed = crate::interactive::session::runtime::SessionRuntime::acquire(
        &session_runtime(),
        None,
        false,
        None,
        id,
        ApprovalPolicy::Ask,
        &root,
    )
    .expect("the resumed session acquires");
    // ...and its grant store is empty: the journal line grants nothing.
    assert!(
        resumed.policy().grants().is_empty(),
        "a resumed session starts with an empty grant store"
    );
    let effect = saya_agent::ToolEffect {
        database_data: false,
        external_side_effect: true,
        requires_approval: true,
        local_state: saya_agent::LocalStateEffect::None,
    };
    assert_eq!(
        resumed.policy().resolve(&effect, Some("sql:analytics")),
        saya_agent::ApprovalDecision::Ask,
        "the journalled grant is not in force: the call the previous process \
         was allowed still asks on resume"
    );
    // And the resume records no new consent: the journal is byte-identical.
    assert_eq!(
        saya_store::SessionJournal::open(root.join(id))
            .read()
            .expect("the journal reads"),
        before,
        "a resume re-grants nothing and re-journals nothing"
    );
    let _ = fs::remove_dir_all(&root);
}
