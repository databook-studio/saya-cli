/// The resume premise the session's own tool description must state (U7):
/// the scratch database lives in the session's state directory and nothing
/// deletes it, so a resumed session — which reuses the record's id and
/// re-enters the same `sessions/<id>/` — re-opens it with its staged tables
/// intact. Deleting it at session end would silently destroy exactly this
/// work; the engine state is durable, and the description must say so.
#[tokio::test]
async fn a_resumed_session_re_enters_its_state_dir_and_reopens_the_scratch() {
    let root = temp_dir("resume-scratch-root");
    let id = "resume-scratch-1";
    // The first process acquires, stages a table, and exits (the runtime
    // drops; the lock releases; no file is removed).
    {
        let first = crate::interactive::session::runtime::SessionRuntime::acquire(
            &session_runtime(None),
            None,
            true,
            None,
            id,
            ApprovalPolicy::Ask,
            &root,
        )
        .expect("the first process acquires");
        first
            .universe()
            .scratch
            .as_ref()
            .expect("scratch composed")
            .run("CREATE TABLE stage AS SELECT 42 AS v")
            .await
            .expect("the first process stages a table");
    }
    // Nothing ended the database: the file is exactly where the state dir
    // put it, with the staged table in it.
    let db = root.join(id).join("scratch.duckdb");
    assert!(
        db.exists(),
        "the scratch database survives the process: {}",
        db.display()
    );
    // The resumed session reuses the same id — the state dir is re-entered,
    // not recreated — and its scratch reads the staged table back.
    let resumed = crate::interactive::session::runtime::SessionRuntime::acquire(
        &session_runtime(None),
        None,
        false,
        None,
        id,
        ApprovalPolicy::Ask,
        &root,
    )
    .expect("the resumed session acquires");
    let rows = resumed
        .universe()
        .scratch
        .as_ref()
        .expect("scratch composed")
        .run("SELECT v FROM stage")
        .await
        .expect("the staged table comes back on resume");
    let values = &rows.rows;
    assert_eq!(
        values.len(),
        1,
        "exactly the staged row comes back: {values:?}"
    );
    let _ = fs::remove_dir_all(&root);
}

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
            &session_runtime(None),
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
        &session_runtime(None),
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
