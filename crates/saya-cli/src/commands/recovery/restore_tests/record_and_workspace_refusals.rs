#[test]
fn restore_refuses_unknown_and_malformed_ids() {
    let sandbox = Sandbox::new("restore-unknown");
    let unknown = format!("ck-{}", "ab".repeat(16));
    assert_eq!(
        restore_change(&sandbox.store, SESSION, Some(&sandbox.ws), &unknown),
        RestoreOutcome::Refused(RestoreRefusal::NotFound)
    );
    assert_eq!(
        restore_change(
            &sandbox.store,
            SESSION,
            Some(&sandbox.ws),
            "not-a-checkpoint"
        ),
        RestoreOutcome::Refused(RestoreRefusal::NotFound)
    );
    assert_eq!(
        restore_change(&sandbox.store, SESSION, Some(&sandbox.ws), ""),
        RestoreOutcome::Refused(RestoreRefusal::NotFound)
    );
}

#[test]
fn restore_without_a_workspace_refuses() {
    let sandbox = Sandbox::new("restore-nows");
    let record = record_commit(
        &sandbox,
        "notes.txt",
        EditSource::WorkspaceWrite,
        None,
        b"x",
        1_000,
    );
    assert_eq!(
        restore_change(&sandbox.store, SESSION, None, record.id.as_str()),
        RestoreOutcome::Refused(RestoreRefusal::NoWorkspace)
    );
}

#[test]
fn restore_refuses_records_that_never_committed() {
    let sandbox = Sandbox::new("restore-uncommitted");
    let prepared = record_prepared(&sandbox, "notes.txt", Some(b"old"), 1_000);
    assert_eq!(
        restore_change(
            &sandbox.store,
            SESSION,
            Some(&sandbox.ws),
            prepared.id.as_str()
        ),
        RestoreOutcome::Refused(RestoreRefusal::NotEligible {
            status: StatusNow::Known(RestoreEligibility::NotCommitted),
            path: "notes.txt".to_owned(),
        })
    );
    sandbox
        .store
        .mark_checkpoint_uncertain(SESSION, prepared.id.as_str(), 2_000)
        .expect("mark");
    assert_eq!(
        restore_change(
            &sandbox.store,
            SESSION,
            Some(&sandbox.ws),
            prepared.id.as_str()
        ),
        RestoreOutcome::Refused(RestoreRefusal::NotEligible {
            status: StatusNow::Known(RestoreEligibility::EffectUncertain),
            path: "notes.txt".to_owned(),
        })
    );
    let records = sandbox.store.list_checkpoints(SESSION).expect("list");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].state, CheckpointState::Uncertain);
}
