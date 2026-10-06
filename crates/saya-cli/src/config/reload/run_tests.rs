//! O04 RED: the wired operation. These fail to compile until `reload_runtime`
//! and `ReloadOutcome` exist in the sibling `run.rs`.

use super::{ReloadOutcome, reload_runtime};
use crate::cli::GlobalOptions;
use crate::config::reload::{ReloadInputs, build_candidate};
use crate::interactive::SessionState;
use std::collections::BTreeMap;
use std::path::PathBuf;

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "saya-reload-run-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn inputs_with(user_dir: PathBuf, process_env: BTreeMap<String, String>) -> ReloadInputs {
    ReloadInputs {
        options: GlobalOptions::default(),
        cwd: temp_dir("cwd"),
        user_dir,
        process_env,
    }
}

fn write_config(user_dir: &std::path::Path, toml: &str) {
    std::fs::write(user_dir.join("config.toml"), toml).unwrap();
}

#[test]
fn unchanged_files_reload_with_the_unchanged_line() {
    let user = temp_dir("unchanged");
    write_config(&user, "[ai]\nmodel = \"m1\"\n[run]\nmax_rows = 50\n");
    let inputs = inputs_with(user.clone(), BTreeMap::new());
    let old = build_candidate(&inputs).unwrap();
    let mut state = SessionState::new("r1", None, "m1");
    let outcome = reload_runtime(&inputs, &old, &mut state);
    match outcome {
        ReloadOutcome::Reloaded { new, text } => {
            assert_eq!(new.resolved.ai.model, "m1");
            assert_eq!(text, "Configuration is unchanged.");
        }
        ReloadOutcome::Failed(text) => panic!("unchanged files must reload, got: {text}"),
    }
}

#[test]
fn an_edited_setting_is_named_in_the_plan_text() {
    let user = temp_dir("edited");
    write_config(&user, "[ai]\nmodel = \"m1\"\n[run]\nmax_rows = 50\n");
    let inputs = inputs_with(user.clone(), BTreeMap::new());
    let old = build_candidate(&inputs).unwrap();
    write_config(&user, "[ai]\nmodel = \"m1\"\n[run]\nmax_rows = 5\n");
    let mut state = SessionState::new("r2", None, "m1");
    match reload_runtime(&inputs, &old, &mut state) {
        ReloadOutcome::Reloaded { text, .. } => {
            assert!(text.contains("max_rows"), "the plan names the key: {text}");
            assert!(text.contains("Applied from the next turn:"), "{text}");
        }
        ReloadOutcome::Failed(text) => panic!("the reload must succeed, got: {text}"),
    }
}

#[test]
fn an_invalid_candidate_fails_with_the_pinned_sentence_and_state_byte_identical() {
    let user = temp_dir("invalid");
    write_config(&user, "[ai]\nmodel = \"m1\"\n");
    let inputs = inputs_with(user.clone(), BTreeMap::new());
    let old = build_candidate(&inputs).unwrap();
    let mut state = SessionState::new("r3", None, "m1");
    let before = serde_json::to_string(&state.redacted()).unwrap();
    write_config(&user, "[run\n");
    match reload_runtime(&inputs, &old, &mut state) {
        ReloadOutcome::Failed(text) => {
            assert!(
                text.starts_with("Reload failed: ")
                    && text.contains("The previous configuration is still in use."),
                "the pinned sentence: {text}"
            );
        }
        ReloadOutcome::Reloaded { text, .. } => {
            panic!("a broken file must fail, got: {text}")
        }
    }
    let after = serde_json::to_string(&state.redacted()).unwrap();
    assert_eq!(before, after, "a failed reload mutates nothing");
}

#[test]
fn a_sharing_widen_reloads_but_never_moves_the_session_value() {
    let user = temp_dir("sharing");
    write_config(&user, "[ai]\nallow_data_sharing = false\n");
    let inputs = inputs_with(user.clone(), BTreeMap::new());
    let old = build_candidate(&inputs).unwrap();
    let mut state = SessionState::new("r4", None, "m");
    state.allow_data_sharing = false;
    write_config(&user, "[ai]\nallow_data_sharing = true\n");
    match reload_runtime(&inputs, &old, &mut state) {
        ReloadOutcome::Reloaded { text, .. } => {
            assert!(
                text.contains("Not applied in this session:")
                    && text.contains("allow_data_sharing"),
                "the widen is classified pending: {text}"
            );
        }
        ReloadOutcome::Failed(text) => panic!("the reload must succeed, got: {text}"),
    }
    assert!(
        !state.allow_data_sharing,
        "the session sharing value never widens"
    );
}
