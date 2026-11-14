/// The `/sessions` subforms map to their own actions: `list` and
/// `delete <id>` carry the id through unchanged — the loops own the
/// behaviour through the shared operations; the adapter owns nothing
/// but the mapping. Bare `/sessions` stays `History`.
#[test]
fn sessions_subforms_map_to_their_actions() {
    let mut state = SessionState::new("test", None, "gpt-4o");
    assert_eq!(
        state.apply(SlashCommand::Sessions, &[]),
        SessionAction::History,
        "bare /sessions keeps today's meaning"
    );
    assert_eq!(
        state.apply(SlashCommand::SessionsList, &[]),
        SessionAction::SessionsList
    );
    assert_eq!(
        state.apply(SlashCommand::SessionsDelete("abc".into()), &[]),
        SessionAction::SessionsDelete("abc".into())
    );
}

/// `/preferences` carries the translated command through unchanged —
/// both loops hand it to the shared `run_in_session` operation scoped
/// to the session's bound root; the adapter owns nothing but the
/// mapping.
#[test]
fn preferences_command_maps_to_the_same_action() {
    use crate::cli::PreferencesCommand;
    let mut state = SessionState::new("test", None, "gpt-4o");
    let command = PreferencesCommand::Add {
        text: "prefer tabs over spaces".into(),
    };
    let action = state.apply(SlashCommand::Preferences(command.clone()), &[]);
    assert_eq!(action, SessionAction::Preferences(command));
}

/// `/rotate` carries through as its own action — the loops own the
/// behaviour through the shared `session_rotate_run` runner; the
/// adapter owns nothing but the mapping.
#[test]
fn rotate_maps_to_its_action() {
    let mut state = SessionState::new("test", None, "gpt-4o");
    assert_eq!(
        state.apply(SlashCommand::Rotate, &[]),
        SessionAction::Rotate
    );
}

/// `/notes` runs the shared `run_notes` operation inside `apply` — the
/// `/tasks clear` shape: apply owns the state, so the loops' normal
/// Message/Error paths emit it and every surface saves on its normal
/// path. Successes are Messages, refusals are Errors, and the state
/// change happens here.
#[test]
fn notes_command_runs_the_shared_operation_in_apply() {
    use crate::interactive::session::lifecycle::notes::NotesCommand;
    use saya_store::StoredContinuation;
    use saya_types::{CONTINUATION_VERSION, SessionContinuationV1, SourceBoundary};
    // A clean set over a typed record: a Message, and the notes stored.
    let mut state = SessionState::new("test", None, "gpt-4o");
    let record = SessionContinuationV1 {
        version: CONTINUATION_VERSION,
        source: SourceBoundary {
            through_turn: 1,
            source_sha256: "c".repeat(64),
            workspace_root: None,
            profile: None,
        },
        narrative: None,
        user_notes: None,
        files: Vec::new(),
        checks: Vec::new(),
    };
    state.continuation = Some(StoredContinuation::Current(record));
    let action = state.apply(
        SlashCommand::Notes(NotesCommand::Set("keep these".into())),
        &[],
    );
    let SessionAction::Message(message) = action else {
        panic!("a clean set is a Message, got {action:?}");
    };
    assert!(message.contains("Session notes stored."), "{message}");
    let Some(StoredContinuation::Current(updated)) = &state.continuation else {
        panic!("the record is still typed");
    };
    assert_eq!(updated.user_notes.as_deref(), Some("keep these"));
    // A clear: a Message, and the notes gone.
    let action = state.apply(SlashCommand::Notes(NotesCommand::Clear), &[]);
    let SessionAction::Message(message) = action else {
        panic!("a clear is a Message, got {action:?}");
    };
    assert!(message.contains("Session notes cleared."), "{message}");
    let Some(StoredContinuation::Current(cleared)) = &state.continuation else {
        panic!("the record is still typed");
    };
    assert_eq!(cleared.user_notes, None);
    // Without a record, both forms refuse as Errors.
    let mut state = SessionState::new("test", None, "gpt-4o");
    for command in [NotesCommand::Set("text".into()), NotesCommand::Clear] {
        let action = state.apply(SlashCommand::Notes(command), &[]);
        let SessionAction::Error(message) = action else {
            panic!("a refusal is an Error, got {action:?}");
        };
        assert!(
            message.contains("run /compact first"),
            "the refusal names the /compact remedy: {message}"
        );
    }
}

/// `/recovery …` passes through `apply` untouched: the loops own the
/// recovery handle (the session's own flag, store and id), so the
/// mapping carries the parsed command verbatim to both surfaces.
#[test]
fn recovery_commands_pass_through_to_the_loops() {
    use crate::commands::recovery::RecoveryCommand;
    let mut state = SessionState::new("test", None, "gpt-4o");
    for command in [
        RecoveryCommand::Status,
        RecoveryCommand::On,
        RecoveryCommand::Off,
        RecoveryCommand::List,
        RecoveryCommand::Diff("ck-abcdef".into()),
        RecoveryCommand::Restore("ck-abcdef1234".into()),
    ] {
        assert_eq!(
            state.apply(SlashCommand::Recovery(command.clone()), &[]),
            SessionAction::Recovery(command),
            "the parsed command is carried verbatim"
        );
    }
}
