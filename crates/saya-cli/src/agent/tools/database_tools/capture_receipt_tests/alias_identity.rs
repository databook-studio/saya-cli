//! A3: the same alias over two different sqlite paths resolves two different
//! `connection_identity` values in the agent-path captures.

use super::super::CaptureEvent;
use super::{e24_connector, e24_root, loop_budget, observing_hook};
use crate::agent::tools::DatabaseTools;
use crate::connection::{ConnectionEntry, ConnectionRegistry};
use crate::interactive::tui::capture::agent_evidence;
use saya_agent::ToolExecutor;
use saya_types::SqlDialect;

#[tokio::test]
async fn same_alias_different_source_yields_different_identity() {
    let dir = e24_root("a3");
    let mut first = ConnectionRegistry::new("analytics");
    first.insert(
        "analytics",
        ConnectionEntry {
            connector: e24_connector(&dir, "first.sqlite3", 1).await,
            dialect: SqlDialect::Sqlite,
            profile_id: Some("e24-identity-first".to_string()),
        },
    );
    let mut second = ConnectionRegistry::new("analytics");
    second.insert(
        "analytics",
        ConnectionEntry {
            connector: e24_connector(&dir, "second.sqlite3", 1).await,
            dialect: SqlDialect::Sqlite,
            profile_id: Some("e24-identity-second".to_string()),
        },
    );
    let ident = |registry: &ConnectionRegistry| {
        registry
            .resolve(Some("analytics"))
            .expect("analytics resolves")
            .profile_id
            .clone()
    };
    let first_id = ident(&first);
    let second_id = ident(&second);
    assert_ne!(
        first_id, second_id,
        "different resolved sources carry different identities"
    );
    for (registry, expected) in [(first, first_id), (second, second_id)] {
        let (hook, observed) = observing_hook();
        let tools = DatabaseTools::with_registry(registry, 100, true, None)
            .with_capture(Some(hook), loop_budget());
        tools
            .execute(
                "bounded_sql_query",
                serde_json::json!({"sql": "SELECT id FROM items"}),
            )
            .await
            .expect("real sqlite agent query succeeds");
        let events = observed.lock().unwrap();
        let CaptureEvent::Captured(capture) = &events[0] else {
            panic!("a small clean result is captured, got {:?}", events[0]);
        };
        let evidence = agent_evidence(capture);
        assert_eq!(
            evidence.connection_identity, expected,
            "the evidence identity follows the resolved source"
        );
    }
    let _ = std::fs::remove_dir_all(dir);
}
