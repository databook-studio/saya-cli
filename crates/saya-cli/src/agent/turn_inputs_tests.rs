//! The session's turn inputs when a configured profile cannot be opened: the
//! session opens without its database rather than not at all.
//!
//! The composition a session runs on is [`prepare_turn`]'s answer: an empty
//! registry is what makes the run a session with no database — the database
//! plugin activates only while `RunContext::database_profiles` is above zero —
//! and the failure it names is what a surface reads to say why.

use saya_config::{ConfigFile, ConnectionsFile, ResolutionInput, resolve};

use super::prepare_turn;
use crate::config::runtime::RuntimeConfig;

/// A runtime whose one profile, `demo`, names a SQLite file that is not there.
fn stale_runtime(label: &str) -> RuntimeConfig {
    let home =
        std::env::temp_dir().join(format!("saya-turn-inputs-{label}-{}", std::process::id()));
    std::fs::create_dir_all(&home).expect("the fixture folder");
    let connections = ConnectionsFile::from_toml(&format!(
        "[profiles.demo]\ntype = 'sqlite'\npath = '{}'\n",
        home.join("gone.sqlite3").display()
    ))
    .expect("the fixture connections parse");
    let resolved = resolve(
        ResolutionInput::new(connections.clone())
            .with_user(ConfigFile::from_toml("[ai]\nmodel = 'test-model'\n").expect("fixture")),
    )
    .expect("the fixture config resolves");
    RuntimeConfig {
        resolved,
        connections,
        config_path: None,
        connections_path: None,
        cache_scope: home.join("cache"),
        investigations_root: home.join("investigations"),
        secret_values: std::collections::BTreeMap::new(),
    }
}

#[tokio::test]
async fn a_profile_that_cannot_open_leaves_the_session_without_a_database() {
    let runtime = stale_runtime("stale");
    let inputs = prepare_turn(
        &runtime,
        &crate::agent::turn_config::PromptOverrides::default(),
        false,
        None,
    )
    .await
    .expect("a session opens without the profile it cannot reach");

    assert!(
        inputs.registry.is_empty(),
        "no connection is attached, so the database plugin never activates: {:?}",
        inputs.registry.names()
    );
    assert_eq!(
        inputs.failures,
        vec![(
            String::from("demo"),
            String::from("connection failed: SQLite connection failed")
        )],
        "the operator is told which profile, and why"
    );
}
