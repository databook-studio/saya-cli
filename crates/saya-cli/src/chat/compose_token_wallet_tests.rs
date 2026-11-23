//! A spent known-token wallet gates the next real tool call.
//!
//! The old body pinned this against production SQLite tools
//! (`agent/runtime_budget_tests/reported_usage.rs`): with
//! `runtime.resolved.limits.session.known_tokens` stated, "reported usage gates
//! real SQLite execution" — one reported token and the SQL call the model asked
//! for runs no query at all, two and it runs.
//!
//! The live path states that ceiling as `TurnPolicy::max_known_tokens`
//! (`plugins/database/turn_policy.rs` projects `[limits.database].known_tokens`
//! into it, and the run arms it on the turn's first database use). This is the
//! same sentence on a composed chat run: once the wallet is spent by an
//! answering call's own report, the SQL call the next answer asks for runs no
//! query at all.
//!
//! **My reading** of the row's "the next real tool call": the live budget arms
//! on the turn's *first database use* (`commands/coding/parent_arming.rs`,
//! `ref/05` §2-§3), so the report of the very answer that first uses the plugin
//! is charged from the arm onwards, not retroactively. The script below asks for
//! two reads: the first is the turn's first database use and executes, and the
//! second — asked by an answer whose report the armed wallet has already spent —
//! is the one gated.

use std::time::Duration;

use saya_agent::ApprovalPolicy;
use saya_agent::{AgentEvent, BudgetDimension};
use serde_json::json;

use super::compose::RunComposition;
use super::compose_fixture::{self, Ceilings};
use super::operator_front_fixture;
use crate::commands::coding::parent_turn_provider::{Reply, Scripted};

/// Load ceiling, not a product bound: how long one scripted turn may take.
const RUN: Duration = Duration::from_secs(120);

/// The one scripted answering call reports ten input and five output tokens, so
/// a one-token wallet is spent by it and a token wallet of that same report is
/// not.
const SPENT_KNOWN_TOKENS: u64 = 1;

/// The front's own sink: every event the turn emitted, in order.
#[derive(Default, Clone)]
struct Recording(std::sync::Arc<std::sync::Mutex<Vec<AgentEvent>>>);

impl Recording {
    fn events(&self) -> Vec<AgentEvent> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn completed_sql(&self) -> usize {
        self.events()
            .iter()
            .filter(|event| {
                matches!(event, AgentEvent::ToolCompleted { name, .. } if name == "bounded_sql_query")
            })
            .count()
    }

    fn requested_sql(&self) -> usize {
        self.events()
            .iter()
            .filter(|event| {
                matches!(event, AgentEvent::ToolRequested { name, .. } if name == "bounded_sql_query")
            })
            .count()
    }

    fn budget_stops(&self) -> Vec<BudgetDimension> {
        self.events()
            .into_iter()
            .filter_map(|event| match event {
                AgentEvent::AnsweringBudgetStopped { dimension } => Some(dimension),
                _ => None,
            })
            .collect()
    }
}

#[async_trait::async_trait]
impl saya_agent::AgentEventSink for Recording {
    async fn emit(&self, event: AgentEvent) {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(event);
    }
}

#[tokio::test]
async fn a_spent_known_token_wallet_gates_the_next_real_tool_call() {
    let provider = Scripted::start_with_script(vec![
        Reply::Call {
            name: "bounded_sql_query",
            arguments: json!({"sql": "SELECT name FROM users ORDER BY id ASC LIMIT 1"}),
        },
        Reply::Call {
            name: "bounded_sql_query",
            arguments: json!({"sql": "SELECT id FROM users ORDER BY id ASC LIMIT 1"}),
        },
        Reply::Text("The users table holds two people, ada and grace."),
    ]);
    let fixture = compose_fixture::in_place_with_database_ceilings(
        "token-wallet",
        &provider.endpoint,
        Ceilings {
            known_tokens: Some(SPENT_KNOWN_TOKENS),
            ..Ceilings::default()
        },
    )
    .await;
    let front = operator_front_fixture::non_interactive();
    let composed = fixture.into_parts(
        front.front(),
        "session-token-wallet",
        ApprovalPolicy::ReadOnly,
    );
    let composition = RunComposition::compose(composed.application, composed.run)
        .await
        .expect("the session's run composes");
    let _root = composed.root;
    composition.open().await.expect("the run opens");
    let sink = Recording::default();

    let turn = tokio::time::timeout(
        RUN,
        composition
            .job()
            .run_next_turn(&sink, "who are the users in the users table?"),
    )
    .await
    .expect("the turn settles");
    assert!(
        turn.is_ok(),
        "a budget stop is a partial turn, not a failure: {turn:?}"
    );
    assert_eq!(
        sink.requested_sql(),
        1,
        "the spent wallet stopped the turn before the second call was made: {:?}",
        sink.events()
    );
    assert_eq!(
        sink.completed_sql(),
        1,
        "reported usage gates real SQLite execution: only the call asked before \
         the armed report ran: {:?}",
        sink.events()
    );
    assert_eq!(
        sink.budget_stops(),
        vec![BudgetDimension::ReportedTokens],
        "the known-token wallet is what stopped it: {:?}",
        sink.events()
    );
    assert_eq!(
        provider.arrived(),
        2,
        "no third request: the paired result of the gated call was never sent"
    );
    assert_eq!(
        crate::chat::ChatOutcome::from_stop(composition.job().turn_stop(), false),
        crate::chat::ChatOutcome::Stopped(crate::chat::Budget {
            dimension: BudgetDimension::ReportedTokens
        }),
        "the surface reads the stop as the token wallet's"
    );
}
