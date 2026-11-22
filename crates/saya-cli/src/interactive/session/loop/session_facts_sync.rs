/// The session's facts, applied: what the chat layer's compaction stated, and
/// the words the state already holds, become the loop's own `SessionState` —
/// through the one writer every compaction has always used — and the cell the
/// next request is rendered from is then re-read from it.
///
/// The boundary states the plan it validated and the summary it saved; this is
/// where they land. A `/notes` edit, a resume's restored record and an
/// unchanged turn all arrive the same way, because the re-read is the state's
/// own answer, never a second copy of the facts.
fn sync_session_facts(
    facts: Option<&crate::chat::SessionFacts>,
    session: &SessionRuntime,
    state: &mut SessionState,
) {
    let Some(facts) = facts else {
        return;
    };
    let digest_of = crate::interactive::session::continuation::digest_resolver(
        session.universe().workspace(),
    );
    facts.apply(state, &digest_of);
}

/// The run's task list, synced into the session record: the run holds the
/// session's own cell, so whatever a turn left there — an accepted `checklist`
/// even when the follow-up request failed — is what the record keeps.
fn sync_session_tasks(
    tasks: &crate::interactive::session::tasks::SessionTasks,
    state: &mut SessionState,
) {
    state.task_list = tasks.current();
}
