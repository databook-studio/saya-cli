/// A saved bypass session carries the mode across the resume — the persisted
/// record is the durable activation fact — and the resume path re-prints the
/// activation line for it, so a user returning to the session reads the
/// mode's own words again rather than a bare `approval:bypass` on the bar.
#[test]
fn a_resumed_session_carries_bypass_and_reprints_the_line() {
    let root = std::env::temp_dir().join(format!("saya-bypass-resume-{}", std::process::id()));
    let store = FsSessionStore::new(&root);
    super::block_on(store.save(RedactedSession {
        version: saya_store::SESSION_VERSION,
        id: "bypassed".into(),
        approval_mode: "bypass".into(),
        ..Default::default()
    }))
    .unwrap();
    let state = load_session(
        &store,
        &cli(),
        &SessionDefaults {
            provider: "ollama".into(),
            model: "m".into(),
            allow_data_sharing: false,
            approval_mode: "ask".into(),
        },
    )
    .unwrap();
    assert_eq!(state.approval_mode, "bypass", "the record carries the mode");
    // Resume continuity keeps it when no explicit flag overrides, and the
    // activation line fires for exactly this mode.
    let kept = crate::interactive::session::turn_loop::resume_approval_mode(
        &GlobalOptions::default(),
        "bypass",
    )
    .unwrap();
    assert_eq!(kept, "bypass");
    assert!(
        crate::interactive::session::permissions::activation::is_bypass_mode(&state),
        "the resumed session is a bypass session"
    );
    let line = crate::interactive::session::permissions::activation::bypass_line(
        &["python3".to_owned()],
        false,
        false,
        &[],
    );
    assert!(
        line.contains("bypass on:") && line.contains("python3"),
        "the line the resume path re-prints is the activation line: {line}"
    );
    std::fs::remove_dir_all(root).unwrap();
}

/// The version-skew direction (DESIGN §7.5): a mode string no binary can
/// parse falls back to `ask` at every parse site — the safe direction — and
/// never into bypass. The parse sites are `unwrap_or(ApprovalPolicy::Ask)`;
/// this pins the fallback's value and the refusal of unknown words, so a
/// typo or a newer mode name degrades to asking, never to running.
#[test]
fn an_unparseable_mode_falls_back_to_ask_never_into_bypass() {
    use saya_agent::ApprovalPolicy;
    assert_eq!(
        ApprovalPolicy::default(),
        ApprovalPolicy::Ask,
        "the mode type's default is the safe direction"
    );
    for unknown in ["bogus", "", "bypass ", "BYPASS", "auto"] {
        assert!(
            unknown.parse::<ApprovalPolicy>().is_err(),
            "`{unknown}` is not a mode: the parse refuses it"
        );
        assert_eq!(
            unknown
                .parse::<ApprovalPolicy>()
                .unwrap_or(ApprovalPolicy::Ask),
            ApprovalPolicy::Ask,
            "the parse sites' fallback is ask, never bypass"
        );
    }
    // A persisted unparseable mode is carried verbatim (resume continuity)
    // and degrades to ask at the parse sites — the state itself never
    // invents a mode.
    let state = super::state_from_redacted(
        RedactedSession {
            version: saya_store::SESSION_VERSION,
            id: "skew".into(),
            approval_mode: "bogus".into(),
            ..Default::default()
        },
        &SessionDefaults {
            provider: "ollama".into(),
            model: "m".into(),
            allow_data_sharing: false,
            approval_mode: "ask".into(),
        },
    );
    assert_eq!(state.approval_mode, "bogus");
    assert!(
        !crate::interactive::session::permissions::activation::is_bypass_mode(&state),
        "an unparseable mode is never bypass"
    );
}
