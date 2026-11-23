//! A SQL call that names no connection is approved as the session's primary.
//!
//! The chat's grants always spoke of the connection a call actually runs on. A
//! call that omits `connection` runs on the primary one, so the token its answer
//! records is `sql:<primary>` — and the next such call runs unasked. Without
//! that, a run's "yes for this run" answers the first no-connection call only,
//! and the same question is put again for every one after it.

use std::time::Duration;

use saya_agent::NoopEventSink;
use serde_json::json;

use super::compose::RunComposition;
use super::compose_fixture::{self, Answering, Composed};
use crate::agent::tools::consent::{Answer, Operator};
use crate::commands::coding::parent_turn_provider::{Reply, Scripted};
use saya_agent::ApprovalPolicy;

/// Load ceiling, not a product bound: how long one scripted turn may take.
const RUN: Duration = Duration::from_secs(120);

/// Test 5: two calls that name no connection, under `ask`, one session answer —
/// the second runs unasked, on the grant the first recorded.
#[tokio::test]
async fn a_call_that_names_no_connection_is_granted_as_the_primary() {
    let provider = Scripted::start_with_script(vec![
        Reply::Call {
            name: "bounded_sql_query",
            arguments: json!({"sql": "SELECT name FROM users ORDER BY id ASC LIMIT 1"}),
        },
        Reply::Call {
            name: "bounded_sql_query",
            arguments: json!({"sql": "SELECT name FROM users ORDER BY id DESC LIMIT 1"}),
        },
        Reply::Text("both queries answered"),
    ]);
    let fixture = compose_fixture::in_place_with_database("sql-primary", &provider.endpoint).await;
    let front = Answering::new(Answer::Session, Answer::Once);
    let Composed {
        application,
        run,
        root,
    } = fixture.into_parts(front.clone(), "session-sql", ApprovalPolicy::Ask);
    let _root = root;
    let composition = RunComposition::compose(application, run)
        .await
        .expect("the session's run composes");
    composition.open().await.expect("the run opens");

    let turn = tokio::time::timeout(
        RUN,
        composition
            .job()
            .run_next_turn(&NoopEventSink, "who are the users in the users table?"),
    )
    .await
    .expect("the turn settles");
    assert!(turn.is_ok(), "the turn must run: {turn:?}");

    assert_eq!(
        front.seen(),
        1,
        "the second no-connection call ran on the primary's grant, unasked"
    );
    let grants = composition
        .operator()
        .grants()
        .expect("the session's operator keeps a run store");
    assert!(
        grants.is_granted("sql:primary"),
        "a session answer to a call that names no connection records `sql:primary`: {:?}",
        grants.tokens()
    );
    let bodies = provider.bodies();
    assert_eq!(bodies.len(), 3, "one request per model call: {bodies:?}");
    assert!(
        bodies[2].contains("grace"),
        "the second query ran: the last row reached the model: {bodies:?}"
    );
}
