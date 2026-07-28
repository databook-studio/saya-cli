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
