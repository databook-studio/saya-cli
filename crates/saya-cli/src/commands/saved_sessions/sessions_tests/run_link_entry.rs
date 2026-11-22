// The three entry points reach one delete: the CLI subcommand's call, the
// slash command's adapter, and the picker's `Delete` command value. Each runs
// on a session with a finished run of its own and removes the record and the
// job; the picker yields the id and nothing else, and the TUI hands it to the
// same adapter the slash command uses.

use crate::front::tui::view::{Picker, SessionCommand};

/// The row the picker lists for `id`: enough for `d` to name it and `y` to
/// yield the command the TUI hands to the one delete.
fn picker_row(id: &str) -> crate::front::tui::view::SessionRow {
    crate::front::tui::view::SessionRow {
        id: id.to_owned(),
        title: "a session".to_owned(),
        database: None,
        turns: 1,
        age: "just now".to_owned(),
        state: crate::front::tui::view::SessionState::Finished,
        read_only: false,
    }
}

#[test]
fn the_three_surfaces_run_the_one_delete() {
    let cli = scratch("link-entry-cli");
    let slash = scratch("link-entry-slash");
    for (sessions, id) in [(&cli, "sess-a"), (&slash, "sess-b")] {
        let jobs = jobs(sessions);
        linked_record(sessions, id, &format!("job-{id}"));
        job_records(
            &jobs,
            &format!("job-{id}"),
            JobOrigin::Session,
            Some("completed"),
        );
        let store = FsSessionStore::new(sessions);
        let outcome = delete_saved_session(&store, sessions, &jobs, id, None);
        assert_eq!(outcome, DeleteOutcome::Deleted, "{id}: {outcome:?}");
        assert!(!sessions.join(format!("{id}.json")).exists(), "{id}");
        assert!(!jobs.join(format!("job-{id}")).exists(), "{id}");
        let _ = fs::remove_dir_all(sessions.parent().unwrap());
    }
    let sessions = scratch("link-entry-picker");
    let base = sessions.parent().unwrap().to_path_buf();
    let jobs = jobs(&sessions);
    linked_record(&sessions, "sess-pick", "job-pick");
    job_records(&jobs, "job-pick", JobOrigin::Session, Some("completed"));
    let mut picker = Picker {
        rows: vec![picker_row("sess-pick")],
        ..Picker::default()
    };
    assert_eq!(picker.ask_delete(), None, "d only asks");
    assert_eq!(
        picker.answer(true),
        Some(SessionCommand::Delete {
            id: "sess-pick".to_owned()
        }),
        "only the answer yields the command"
    );
    let store = FsSessionStore::new(&sessions);
    let (ok, message) = crate::interactive::session::commands::sessions_delete_text(
        &store,
        &sessions,
        &jobs,
        "sess-pick",
        None,
    );
    assert!(ok, "{message}");
    assert!(!sessions.join("sess-pick.json").exists(), "{message}");
    assert!(
        !jobs.join("job-pick").exists(),
        "the picker's delete runs the one operation"
    );
    let _ = fs::remove_dir_all(base);
}
