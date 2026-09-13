use super::{SessionDefaults, load_session};
use crate::{Cli, GlobalOptions};
use saya_store::{FsSessionStore, RedactedMessage, RedactedSession, SessionStore};

fn cli() -> Cli {
    Cli {
        options: GlobalOptions {
            continue_session: true,
            ..Default::default()
        },
        command: None,
    }
}

#[test]
fn v1_session_uses_current_runtime_defaults() {
    let root = std::env::temp_dir().join(format!("saya-v1-{}", std::process::id()));
    let store = FsSessionStore::new(&root);
    super::block_on(store.save(RedactedSession {
        version: 1,
        id: "legacy".into(),
        profile_names: vec!["analytics".into()],
        messages: vec![],
        ..Default::default()
    }))
    .unwrap();
    let state = load_session(
        &store,
        &cli(),
        &SessionDefaults {
            provider: "openai_compatible".into(),
            model: "current-model".into(),
            allow_data_sharing: true,
            approval_mode: "read-only".into(),
        },
    )
    .unwrap();
    assert_eq!(state.provider, "openai_compatible");
    assert_eq!(state.model, "current-model");
    assert!(state.allow_data_sharing);
    assert_eq!(state.approval_mode, "read-only");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn raw_legacy_json_without_version_uses_current_privacy_defaults() {
    let root = std::env::temp_dir().join(format!("saya-raw-v1-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("raw.json"),
        r#"{"id":"raw","profile_names":[],"messages":[]}"#,
    )
    .unwrap();
    let state = load_session(
        &FsSessionStore::new(&root),
        &cli(),
        &SessionDefaults {
            provider: "openai".into(),
            model: "runtime-model".into(),
            allow_data_sharing: true,
            approval_mode: "read-only".into(),
        },
    )
    .unwrap();
    assert_eq!(state.provider, "openai");
    assert_eq!(state.model, "runtime-model");
    assert!(state.allow_data_sharing);
    assert_eq!(state.approval_mode, "read-only");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn v2_session_restores_persisted_settings() {
    let root = std::env::temp_dir().join(format!("saya-v2-{}", std::process::id()));
    let store = FsSessionStore::new(&root);
    super::block_on(store.save(RedactedSession {
        version: saya_store::SESSION_VERSION,
        id: "saved".into(),
        provider: "ollama".into(),
        model: "saved-model".into(),
        allow_data_sharing: false,
        approval_mode: "never".into(),
        ..Default::default()
    }))
    .unwrap();
    let state = load_session(
        &store,
        &cli(),
        &SessionDefaults {
            provider: "openai".into(),
            model: "current-model".into(),
            allow_data_sharing: true,
            approval_mode: "ask".into(),
        },
    )
    .unwrap();
    assert_eq!(state.provider, "ollama");
    assert_eq!(state.model, "saved-model");
    assert!(!state.allow_data_sharing);
    assert_eq!(state.approval_mode, "never");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn legacy_messages_migrate_to_one_safe_turn() {
    let root = std::env::temp_dir().join(format!("saya-migrate-{}", std::process::id()));
    let store = FsSessionStore::new(&root);
    super::block_on(store.save(RedactedSession {
        version: 1,
        id: "legacy-messages".into(),
        messages: vec![
            RedactedMessage {
                role: "system".into(),
                content: "old command".into(),
            },
            RedactedMessage {
                role: "user".into(),
                content: "question".into(),
            },
            RedactedMessage {
                role: "assistant".into(),
                content: "answer".into(),
            },
        ],
        ..Default::default()
    }))
    .unwrap();
    let state = load_session(
        &store,
        &cli(),
        &SessionDefaults {
            provider: "ollama".into(),
            model: "model".into(),
            allow_data_sharing: false,
            approval_mode: "ask".into(),
        },
    )
    .unwrap();
    assert_eq!(state.turns.len(), 1);
    assert_eq!(state.provider_history().len(), 2);
    std::fs::remove_dir_all(root).unwrap();
}

/// M0-2: persist with `read-only`, resume with an explicit
/// `--approval-mode never` — the flag must override the persisted mode
/// instead of being silently ignored.
#[test]
fn resume_honors_an_explicit_approval_mode_override() {
    let root = std::env::temp_dir().join(format!("saya-resume-override-{}", std::process::id()));
    let store = FsSessionStore::new(&root);
    super::block_on(store.save(RedactedSession {
        version: saya_store::SESSION_VERSION,
        id: "override".into(),
        approval_mode: "read-only".into(),
        ..Default::default()
    }))
    .unwrap();
    let cli = Cli {
        options: GlobalOptions {
            continue_session: true,
            approval_mode: Some("never".into()),
            ..Default::default()
        },
        command: None,
    };
    let mut state = load_session(
        &store,
        &cli,
        &SessionDefaults {
            provider: "openai".into(),
            model: "current-model".into(),
            allow_data_sharing: true,
            approval_mode: "never".into(),
        },
    )
    .unwrap();
    // Loading alone keeps resume continuity: the persisted mode stands...
    assert_eq!(state.approval_mode, "read-only");
    // ...and the loop's resume resolution lets the explicit flag win.
    state.approval_mode =
        super::super::session_loop::resume_approval_mode(&cli.options, &state.approval_mode)
            .unwrap();
    assert_eq!(state.approval_mode, "never");
    std::fs::remove_dir_all(root).unwrap();
}

/// M0-2: resume without the flag keeps the persisted mode (resume
/// continuity).
#[test]
fn resume_without_the_flag_keeps_the_persisted_mode() {
    let root = std::env::temp_dir().join(format!("saya-resume-keep-{}", std::process::id()));
    let store = FsSessionStore::new(&root);
    super::block_on(store.save(RedactedSession {
        version: saya_store::SESSION_VERSION,
        id: "keep".into(),
        approval_mode: "read-only".into(),
        ..Default::default()
    }))
    .unwrap();
    let cli = cli();
    let mut state = load_session(
        &store,
        &cli,
        &SessionDefaults {
            provider: "openai".into(),
            model: "current-model".into(),
            allow_data_sharing: true,
            approval_mode: "ask".into(),
        },
    )
    .unwrap();
    assert_eq!(state.approval_mode, "read-only");
    // The resume resolution without the flag keeps the persisted mode.
    state.approval_mode =
        super::super::session_loop::resume_approval_mode(&cli.options, &state.approval_mode)
            .unwrap();
    assert_eq!(state.approval_mode, "read-only");
    std::fs::remove_dir_all(root).unwrap();
}

/// An older session file written before the `arguments` and `result_shape`
/// fields existed still loads: the new fields are `#[serde(default)]`, so a
/// tool record carrying only `name` and `status` deserializes with empty
/// arguments and a `None` shape. Old and new session files interoperate.
#[test]
fn an_old_session_file_without_the_new_tool_fields_still_loads() {
    let root = std::env::temp_dir().join(format!("saya-old-tool-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("old.json"),
        r#"{"version":2,"id":"old","profile_names":["analytics"],"turns":[{"user":"q","assistant":"a","database_derived":true,"tools":[{"name":"bounded_sql_query","status":"completed"}]}],"messages":[]}"#,
    )
    .unwrap();
    let state = load_session(
        &FsSessionStore::new(&root),
        &cli(),
        &SessionDefaults {
            provider: "ollama".into(),
            model: "m".into(),
            allow_data_sharing: false,
            approval_mode: "ask".into(),
        },
    )
    .unwrap();
    let tool = &state.turns[0].tools[0];
    assert_eq!(tool.name, "bounded_sql_query");
    assert_eq!(tool.status, "completed");
    assert_eq!(tool.arguments, "", "missing arguments default to empty");
    assert!(
        tool.result_shape.is_none(),
        "missing result_shape defaults to None"
    );
    std::fs::remove_dir_all(root).unwrap();
}

/// The resume pin: a session whose record carries a workspace root re-opens
/// that root on resume, whatever directory the resume happens from — the
/// root follows the record, not the shell. A session written before the
/// workspace existed carries no root and resumes unbound, exactly its old
/// behaviour.
#[test]
fn the_recorded_workspace_root_rides_the_resume() {
    let root = std::env::temp_dir().join(format!("saya-ws-pin-{}", std::process::id()));
    let store = FsSessionStore::new(&root);
    super::block_on(store.save(RedactedSession {
        version: saya_store::SESSION_VERSION,
        id: "pinned".into(),
        workspace_root: Some("/projects/saya".into()),
        ..Default::default()
    }))
    .unwrap();
    let state = load_session(
        &store,
        &cli(),
        &SessionDefaults {
            provider: "openai".into(),
            model: "current-model".into(),
            allow_data_sharing: true,
            approval_mode: "ask".into(),
        },
    )
    .unwrap();
    assert_eq!(
        state.workspace_root.as_deref(),
        Some("/projects/saya"),
        "the pin follows the record, not the resume cwd"
    );
    // A record written before the workspace existed: no root, unbound.
    super::block_on(store.save(RedactedSession {
        version: saya_store::SESSION_VERSION,
        id: "legacy".into(),
        ..Default::default()
    }))
    .unwrap();
    let legacy = Cli {
        options: GlobalOptions {
            resume: Some("legacy".into()),
            ..Default::default()
        },
        command: None,
    };
    let old = load_session(
        &store,
        &legacy,
        &SessionDefaults {
            provider: "openai".into(),
            model: "current-model".into(),
            allow_data_sharing: true,
            approval_mode: "ask".into(),
        },
    )
    .unwrap();
    assert!(
        old.workspace_root.is_none(),
        "no record, no pin: the old session resumes unbound"
    );
    let _ = std::fs::remove_dir_all(root);
}

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
    let kept =
        super::super::session_loop::resume_approval_mode(&GlobalOptions::default(), "bypass")
            .unwrap();
    assert_eq!(kept, "bypass");
    assert!(
        crate::interactive::session_activation::is_bypass_mode(&state),
        "the resumed session is a bypass session"
    );
    let line = crate::interactive::session_activation::bypass_line(&["python3".to_owned()], false);
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
        !crate::interactive::session_activation::is_bypass_mode(&state),
        "an unparseable mode is never bypass"
    );
}
