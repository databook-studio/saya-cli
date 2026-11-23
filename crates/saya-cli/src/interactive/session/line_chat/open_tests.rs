//! The session's own word when the configured profile cannot be opened: the one
//! composition both surfaces pass through says it once, and the run opens
//! anyway — a session with no database, never a session that cannot start.
//!
//! The surface here is the line front's own: a [`ChatFront`] that keeps what it
//! was told, so what the plain loop would print is assertable without a
//! terminal.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use saya_agent::{AgentEventSink, ApprovalPolicy, NoopEventSink};
use saya_store::SqliteStateStore;

use super::open::{LineSession, session_run};
use crate::GlobalOptions;
use crate::agent::tools::consent::Answer;
use crate::chat::{ChatEvent, ChatFront, Question};
use crate::config::runtime::{RuntimeConfig, load_with_sources};
use crate::plugins::database::DatabaseBudget;
use crate::render::RenderFormat;

/// A front that records what the session said and never asks anybody.
#[derive(Default)]
struct Recording {
    events: Mutex<Vec<String>>,
}

#[async_trait]
impl ChatFront for Recording {
    fn sink(&self) -> Arc<dyn AgentEventSink> {
        Arc::new(NoopEventSink)
    }

    async fn ask(&self, _question: &Question) -> Answer {
        Answer::Deny
    }

    fn emit(&self, event: ChatEvent) {
        if let ChatEvent::Notice { message } = event {
            self.events
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(message);
        }
    }
}

/// A folder and a configuration this test owns, whose one profile names a
/// SQLite file that is not there.
fn stale_session(label: &str) -> (PathBuf, LineSession) {
    let root = std::env::temp_dir().join(format!(
        "saya-line-chat-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or(0)
    ));
    let folder = root.join("folder");
    std::fs::create_dir_all(&folder).unwrap();
    let config_home = root.join("config").join("saya");
    std::fs::create_dir_all(&config_home).unwrap();
    std::fs::write(config_home.join("config.toml"), "# this row's own home\n").unwrap();
    let project = root.join("project.toml");
    std::fs::write(&project, "[ai]\nmodel = 'test-model'\n").unwrap();
    let connections = root.join("connections.toml");
    std::fs::write(
        &connections,
        format!(
            "[profiles.demo]\ntype = 'sqlite'\npath = '{}'\n",
            root.join("gone.sqlite3").display()
        ),
    )
    .unwrap();
    let options = GlobalOptions {
        config: Some(project),
        connections: Some(connections),
        ..GlobalOptions::default()
    };
    let runtime: RuntimeConfig =
        load_with_sources(&options, &folder, &config_home, BTreeMap::new())
            .expect("the row's runtime resolves");
    let store = SqliteStateStore::new(root.join("state.sqlite3"));
    let parts = LineSession {
        session: String::from("stale-profile-row"),
        folder: Some(folder.clone()),
        runtime: Arc::new(runtime),
        store,
        mode: ApprovalPolicy::Ask,
        overrides: crate::agent::turn_config::PromptOverrides::default(),
        format: RenderFormat::Text,
        sink: None,
        budget: DatabaseBudget::defaults(),
        can_prompt: false,
        grants: saya_agent::SessionGrants::default(),
        state_root: Some(root.join("coding")),
        cancellation: None,
        tasks: None,
        facts: None,
        seed: Vec::new(),
    };
    (root, parts)
}

#[tokio::test]
async fn a_profile_that_cannot_open_is_said_once_and_the_run_opens() {
    let (root, parts) = stale_session("stale");
    let front = Arc::new(Recording::default());
    let run = session_run(
        &parts,
        Arc::clone(&front) as Arc<dyn ChatFront>,
        String::from("stale-profile-row"),
        &crate::chat::WorkspaceShape::Writable,
    )
    .await
    .expect("the run opens without the profile it cannot reach");

    let said = front
        .events
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    assert_eq!(
        said,
        vec![String::from(
            "cannot open profile 'demo': connection failed: SQLite connection failed"
        )],
        "one line, naming the profile and the error"
    );
    assert!(
        run.connections.is_empty(),
        "no connection is attached: the run is a session with no database"
    );
    assert_eq!(
        run.skipped_primary.as_deref(),
        Some("demo"),
        "the surface is told which profile it is not talking to"
    );
    drop(root);
}
