//! The plugin's query capture: a chat run's successful `bounded_sql_query`
//! captures exactly what the old chat turn captured; a refused capture carries
//! its reason and no rows; the fan-out and a job surface never capture; and a
//! plugin built without a hook behaves exactly as before.

use std::sync::Arc;

use saya_agent::RunPlugin;
use saya_types::{QueryResult, SqlDialect};

use super::DatabasePlugin;
use super::capture_fixture::{
    OversizedCellConnector, PROFILE_IDENTITY, chat_plugin, observing_hook, sqlite_entry,
};
use super::tests::{job_tools, scratch};
use crate::agent::tools::{CaptureEvent, CaptureRefusalReason};
use crate::connection::ConnectionEntry;

/// One successful query through the plugin's own executor: the hook receives
/// exactly one capture, holding the rows the model saw and the connection that
/// ran them.
#[tokio::test]
async fn a_chat_plugin_query_captures_the_rows_and_the_connection() {
    let (hook, observed) = observing_hook();
    let plugin = chat_plugin(sqlite_entry("capture-rows").await, Some(hook));
    let executor = plugin.executor().expect("the chat plugin serves its tools");

    let value = executor
        .execute(
            "bounded_sql_query",
            serde_json::json!({"connection": "primary", "sql": "SELECT id, name FROM users ORDER BY id"}),
        )
        .await
        .expect("the query succeeds");

    let events = observed.lock().unwrap();
    assert_eq!(events.len(), 1, "exactly one capture: {events:?}");
    let CaptureEvent::Captured(capture) = &events[0] else {
        panic!("expected a capture, got {:?}", events[0]);
    };
    assert_eq!(capture.connection, "primary");
    assert_eq!(capture.profile_identity.as_deref(), Some(PROFILE_IDENTITY));
    let model_result: QueryResult = serde_json::from_value(value).expect("a query result");
    assert_eq!(capture.result, model_result, "the rows the model received");
    assert_eq!(
        capture.result.rows.len(),
        50,
        "the model's row cap, not the configured 200"
    );
}

/// A query whose model view the capture path refuses emits the refusal reason
/// with the statement and connection — never rows.
#[tokio::test]
async fn a_refused_capture_carries_the_reason_not_rows() {
    let (hook, observed) = observing_hook();
    let entry = ConnectionEntry {
        connector: Box::new(OversizedCellConnector),
        dialect: SqlDialect::Sqlite,
        profile_id: None,
    };
    let plugin = chat_plugin(entry, Some(hook));
    let executor = plugin.executor().expect("the chat plugin serves its tools");

    let value = executor
        .execute(
            "bounded_sql_query",
            serde_json::json!({"connection": "primary", "sql": "SELECT big"}),
        )
        .await
        .expect("the model still receives its result");
    assert_eq!(value["columns"][0], "big", "the model's value is unchanged");

    let events = observed.lock().unwrap();
    assert_eq!(events.len(), 1, "exactly one event: {events:?}");
    assert!(
        matches!(
            &events[0],
            CaptureEvent::Refused {
                sql,
                connection,
                reason: CaptureRefusalReason::ModelViewTruncated,
            } if sql == "SELECT big" && connection == "primary"
        ),
        "the refusal names its reason, never the rows: {:?}",
        events[0]
    );
}

/// The fan-out runs one query per database through the same tools that carry
/// the hook: it must never capture — only single-connection
/// `bounded_sql_query` is preview evidence.
#[tokio::test]
async fn the_fan_out_never_captures() {
    let (hook, observed) = observing_hook();
    let plugin = chat_plugin(sqlite_entry("capture-fanout").await, Some(hook));
    let executor = plugin.executor().expect("the chat plugin serves its tools");

    let value = executor
        .execute(
            "bounded_sql_query_all",
            serde_json::json!({"sql": "SELECT count(*) FROM users"}),
        )
        .await
        .expect("the fan-out succeeds");
    assert!(
        value["databases"]
            .as_array()
            .is_some_and(|all| all.len() == 1),
        "the fan-out still answers per database: {value}"
    );
    assert!(
        observed.lock().unwrap().is_empty(),
        "the fan-out never captures"
    );
}

/// A plugin built without a hook behaves exactly as it did before: the same
/// query returns the same value, and only a plugin that was given a hook
/// captures.
#[tokio::test]
async fn a_plugin_built_without_a_hook_behaves_as_before() {
    let (hook, observed) = observing_hook();
    let plain = chat_plugin(sqlite_entry("capture-plain").await, None);
    let hooked = chat_plugin(sqlite_entry("capture-hooked").await, Some(hook));
    let query =
        serde_json::json!({"connection": "primary", "sql": "SELECT id FROM users ORDER BY id"});

    let plain_value = plain
        .executor()
        .expect("the chat plugin serves its tools")
        .execute("bounded_sql_query", query.clone())
        .await
        .expect("the plain path succeeds");
    let hooked_value = hooked
        .executor()
        .expect("the chat plugin serves its tools")
        .execute("bounded_sql_query", query)
        .await
        .expect("the hooked path succeeds");

    assert_eq!(plain_value, hooked_value, "the model's value is identical");
    assert_eq!(
        observed.lock().unwrap().len(),
        1,
        "only the hooked plugin captured"
    );
}

/// A job surface's queries are the job's own evidence, never a preview: even
/// with the run's hook attached to the plugin, nothing is captured.
#[tokio::test]
async fn a_job_surface_never_captures() {
    let (hook, observed) = observing_hook();
    let root = scratch("capture-job");
    let plugin = DatabasePlugin::for_job(Arc::new(job_tools(&root, false)), None)
        .with_test_capture(Some(hook));
    let executor = plugin.executor().expect("the job plugin serves its tools");

    let value = executor
        .execute(
            "bounded_sql_query",
            serde_json::json!({"connection": "main", "sql": "SELECT 1 AS one"}),
        )
        .await
        .expect("the job's query succeeds");
    assert_eq!(value["columns"][0], "one", "the job still gets its rows");
    assert!(
        observed.lock().unwrap().is_empty(),
        "a job surface never captures a query"
    );
}
