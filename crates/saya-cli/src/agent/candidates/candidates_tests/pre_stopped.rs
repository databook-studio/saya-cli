//! A question stopped before it starts opens no setup connection.
//!
//! The old body pinned this from inside its candidate entry
//! (`agent/runtime_timer_tests/pre_stopped.rs`): a cancelled question must reach
//! neither a source nor a `saya ask` candidate attempt — "a stopped question
//! must not open a new setup connection ({candidates} candidates)".
//!
//! The live path states the same rule at the entry that survives:
//! [`super::run_with_candidates`] reads the question's own stop before anything
//! is built, so neither the one attempt nor the multi-candidate executor's
//! setup connection is opened. The source here is a PostgreSQL profile whose
//! server is a bound, silent socket: any accepted connection is visible.

use std::collections::BTreeMap;
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use saya_agent::{
    AgentEvent, AgentEventSink, ApprovalPolicy, CancellationReason, CancellationToken,
};
use saya_config::{ConnectionsFile, ResolutionInput, resolve};
use saya_types::DatabaseProfile;

use super::super::run_with_candidates;
use crate::agent::question_scope::QuestionScope;
use crate::agent::turn_config::PromptOverrides;
use crate::config::runtime::RuntimeConfig;
use crate::render::RenderFormat;

/// A sink that keeps every event, so "no event reached the surface" is
/// assertable.
#[derive(Default)]
struct Recording(Arc<Mutex<Vec<AgentEvent>>>);

#[async_trait::async_trait]
impl AgentEventSink for Recording {
    async fn emit(&self, event: AgentEvent) {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(event);
    }
}

/// A runtime whose one profile is a PostgreSQL server that accepts the socket
/// and never answers — so a setup connection is the one thing it would do.
fn stalled_runtime(root: &std::path::Path) -> RuntimeConfig {
    let profile = DatabaseProfile::Postgres {
        host: "127.0.0.1".into(),
        port: Some(0),
        database: "synthetic".into(),
        user: "saya".into(),
        ssl_mode: Some(saya_types::PostgresSslMode::Disable),
        password: None,
    };
    let connections = ConnectionsFile {
        profiles: BTreeMap::from([("analytics".into(), profile.clone())]),
    };
    let mut runtime = RuntimeConfig {
        resolved: resolve(ResolutionInput::new(connections.clone())).expect("test config resolves"),
        connections,
        config_path: None,
        connections_path: Some(root.join("connections.toml")),
        cache_scope: root.to_path_buf(),
        investigations_root: root.join("investigations"),
        secret_values: BTreeMap::new(),
    };
    runtime.resolved.profile_name = Some("analytics".into());
    runtime.resolved.profile = Some(profile);
    runtime.resolved.query_timeout_seconds = 1;
    runtime
}

fn scratch(name: &str) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target")
        .join(format!("tmp-candidates-prestop-{name}"));
    std::fs::create_dir_all(&root).expect("the fixture root creates");
    root
}

#[tokio::test]
async fn a_question_stopped_before_it_starts_opens_no_setup_connection() {
    for candidates in [1, 2] {
        let root = scratch(&format!("{candidates}"));
        let silent = TcpListener::bind("127.0.0.1:0").expect("the silent server binds");
        silent
            .set_nonblocking(true)
            .expect("the silent server is non-blocking");
        let port = silent
            .local_addr()
            .expect("the silent server's address")
            .port();
        let mut runtime = stalled_runtime(&root);
        runtime.resolved.profile = match runtime.resolved.profile.clone() {
            Some(DatabaseProfile::Postgres {
                host,
                database,
                user,
                ssl_mode,
                password,
                ..
            }) => Some(DatabaseProfile::Postgres {
                host,
                port: Some(port),
                database,
                user,
                ssl_mode,
                password,
            }),
            other => other,
        };
        let cancellation = CancellationToken::new();
        cancellation.cancel_with_reason(CancellationReason::User);
        let scope = QuestionScope::session(&runtime, cancellation.clone());
        let sink = Arc::new(Recording::default());

        let result = tokio::time::timeout(
            Duration::from_secs(10),
            run_with_candidates(
                &runtime,
                "do not start a stopped question",
                ApprovalPolicy::ReadOnly,
                false,
                PromptOverrides::default(),
                root.clone(),
                RenderFormat::Text,
                Arc::clone(&sink) as Arc<dyn AgentEventSink>,
                scope,
                None,
                candidates,
            ),
        )
        .await
        .expect("a stopped question settles without source work");
        assert!(
            result.is_err(),
            "a stopped question is refused, not run ({candidates} candidates)"
        );
        assert!(
            matches!(silent.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock),
            "a stopped question must not open a new setup connection ({candidates} candidates)"
        );
        assert!(
            sink.0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_empty(),
            "no attempt reached the surface ({candidates} candidates)"
        );
        assert_eq!(
            cancellation.reason(),
            Some(CancellationReason::User),
            "the question's own stop is untouched ({candidates} candidates)"
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
