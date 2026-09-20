/// Session grants: the primary token offer, cross-turn force, and narrowness to one shape.
/// Moved byte-identical from the hub; no snapshots involved.
use super::super::{ChannelApproval, StreamMsg};
use super::support::{composed_facts, read_shaped_tool, side_effecting_tool};
use crate::grant_token::TurnPrimary;
use saya_agent::{ApprovalChoice, ApprovalDecider, ApprovalPolicy, SessionPolicy};
use tokio::sync::mpsc::unbounded_channel;

/// The TUI's modal offers the SQL family's token the way the turn binds it:
/// a call naming no connection suggests `sql:<primary>` — the primary's real
/// registry name, bound into the decider by the turn — and the recorded
/// grant pre-answers the next call of the same connection.
#[tokio::test]
async fn the_tui_s_modal_offers_the_turn_s_primary_sql_token() {
    let primary = TurnPrimary::default();
    primary.bind(&crate::grant_token_tests::registry_with_primary(
        "analytics",
    ));
    let (tx, mut rx) = unbounded_channel();
    let decider = ChannelApproval::new(
        tx,
        SessionPolicy::new(ApprovalPolicy::Ask),
        primary.clone(),
        crate::approval_facts::ApprovalFacts::default(),
        None,
    );
    let answerer = tokio::spawn(async move {
        if let Some(StreamMsg::ApprovalRequest { respond, grant, .. }) = rx.recv().await {
            assert_eq!(
                grant.as_deref(),
                Some("sql:analytics"),
                "a connectionless SQL call offers the primary's real registry name"
            );
            let _ = respond.send(ApprovalChoice::AllowSession {
                token: grant.expect("the ask offered a token"),
            });
        }
    });
    let sql = read_shaped_tool();
    assert!(
        decider
            .approve(&sql, &serde_json::json!({"sql": "SELECT 1"}))
            .await,
        "the user's session grant allows the call that asked"
    );
    answerer.await.expect("the answerer completes");
    // And an unbound handle — a decider built before its turn bound a
    // registry (or a run's construction) — offers no token at all.
    let (tx, mut rx) = unbounded_channel();
    let decider = ChannelApproval::new(
        tx,
        SessionPolicy::new(ApprovalPolicy::Ask),
        TurnPrimary::default(),
        crate::approval_facts::ApprovalFacts::default(),
        None,
    );
    let answerer = tokio::spawn(async move {
        if let Some(StreamMsg::ApprovalRequest { grant, .. }) = rx.recv().await {
            assert_eq!(
                grant, None,
                "an unbound primary suggests no token — fail closed, never a \
                 guessed name"
            );
        }
    });
    let _ = decider
        .approve(&sql, &serde_json::json!({"sql": "SELECT 1"}))
        .await;
    answerer.await.expect("the answerer completes");
}

/// The grant outlives the turn: a session grant recorded through one turn's
/// decider is in force for a later turn's decider. Each turn builds its
/// decider over a clone of the one session policy, and a clone shares the
/// grant store — so turn four resolves the granted shape with no ask at all.
#[tokio::test]
async fn a_tui_grant_made_in_one_turn_is_in_force_in_the_next() {
    let tool = side_effecting_tool();
    let arguments = serde_json::json!({"program": "bench"});
    let policy = SessionPolicy::new(ApprovalPolicy::Ask);
    // Turn three: the ask arrives with the suggested token, and the user's
    // session grant is recorded through the decider.
    let (tx, mut rx) = unbounded_channel();
    let turn_three = ChannelApproval::new(
        tx,
        policy.clone(),
        TurnPrimary::default(),
        composed_facts(),
        None,
    );
    let answerer = tokio::spawn(async move {
        if let Some(StreamMsg::ApprovalRequest { respond, grant, .. }) = rx.recv().await {
            assert_eq!(
                grant.as_deref(),
                Some("runner:bench"),
                "the ask must offer the token the grant would record"
            );
            let _ = respond.send(ApprovalChoice::AllowSession {
                token: grant.expect("the ask offered a token"),
            });
        }
    });
    assert!(
        turn_three.approve(&tool, &arguments).await,
        "the user's session grant allows the call that asked"
    );
    answerer.await.expect("the answerer completes");
    // Turn four: a fresh decider over the same session policy — the wiring
    // `start` uses. The same shape resolves Allow with no ask: the channel is
    // closed, so an ask would deny.
    let (tx, rx) = unbounded_channel();
    drop(rx);
    let turn_four = ChannelApproval::new(
        tx,
        policy.clone(),
        TurnPrimary::default(),
        composed_facts(),
        None,
    );
    assert!(
        turn_four.approve(&tool, &arguments).await,
        "the grant made in turn three is still in force in turn four"
    );
    assert!(
        policy.grants().is_granted("runner:bench"),
        "the grant was recorded into the one session policy"
    );
}

/// A grant is narrow: `runner:bench` granted does not answer `runner:deploy`,
/// and `fetch:https+a.example` granted does not answer `fetch:https+b.example`
/// — a different shape still asks.
#[tokio::test]
async fn a_grant_does_not_answer_a_different_shape() {
    let policy = SessionPolicy::new(ApprovalPolicy::Ask);
    let granted = side_effecting_tool();
    let granted_args = serde_json::json!({"program": "bench"});
    let other = side_effecting_tool();
    let other_args = serde_json::json!({"program": "deploy"});

    // Turn three grants runner:bench through the ask.
    let (tx, mut rx) = unbounded_channel();
    let decider = ChannelApproval::new(
        tx,
        policy.clone(),
        TurnPrimary::default(),
        composed_facts(),
        None,
    );
    let answerer = tokio::spawn(async move {
        if let Some(StreamMsg::ApprovalRequest { respond, grant, .. }) = rx.recv().await {
            assert_eq!(grant.as_deref(), Some("runner:bench"));
            let _ = respond.send(ApprovalChoice::AllowSession {
                token: grant.expect("the ask offered a token"),
            });
        }
    });
    assert!(decider.approve(&granted, &granted_args).await);
    answerer.await.expect("the answerer completes");

    // The granted shape is pre-answered (closed channel: an ask would deny).
    let (tx, rx) = unbounded_channel();
    drop(rx);
    let decider = ChannelApproval::new(
        tx,
        policy.clone(),
        TurnPrimary::default(),
        composed_facts(),
        None,
    );
    assert!(
        decider.approve(&granted, &granted_args).await,
        "the granted shape runs without asking"
    );
    // A different shape still asks — and a deny there denies.
    let (tx, mut rx) = unbounded_channel();
    let decider = ChannelApproval::new(
        tx,
        policy.clone(),
        TurnPrimary::default(),
        composed_facts(),
        None,
    );
    let answerer = tokio::spawn(async move {
        if let Some(StreamMsg::ApprovalRequest { respond, grant, .. }) = rx.recv().await {
            assert_eq!(grant.as_deref(), Some("runner:deploy"));
            let _ = respond.send(ApprovalChoice::Deny);
        }
    });
    assert!(
        !decider.approve(&other, &other_args).await,
        "`runner:bench` granted does not allow `runner:deploy`"
    );
    answerer.await.expect("the answerer completes");
    assert!(
        !policy.grants().is_granted("runner:deploy"),
        "a deny grants nothing"
    );

    // The same narrowness for fetch: one host's grant never covers another.
    let fetch = crate::interactive::session_definitions::http_fetch();
    let here = serde_json::json!({"url": "https://a.example/x"});
    let there = serde_json::json!({"url": "https://b.example/x"});
    let (tx, mut rx) = unbounded_channel();
    let decider = ChannelApproval::new(
        tx,
        policy.clone(),
        TurnPrimary::default(),
        composed_facts(),
        None,
    );
    let answerer = tokio::spawn(async move {
        if let Some(StreamMsg::ApprovalRequest { respond, grant, .. }) = rx.recv().await {
            assert_eq!(grant.as_deref(), Some("fetch:https+a.example"));
            let _ = respond.send(ApprovalChoice::AllowSession {
                token: grant.expect("the ask offered a token"),
            });
        }
    });
    assert!(decider.approve(&fetch, &here).await, "a.example is granted");
    answerer.await.expect("the answerer completes");
    let (tx, rx) = unbounded_channel();
    drop(rx);
    let decider = ChannelApproval::new(
        tx,
        policy.clone(),
        TurnPrimary::default(),
        composed_facts(),
        None,
    );
    assert!(
        !decider.approve(&fetch, &there).await,
        "`fetch:https+a.example` does not allow `fetch:https+b.example`: it asks, \
         and nobody answers"
    );
}
