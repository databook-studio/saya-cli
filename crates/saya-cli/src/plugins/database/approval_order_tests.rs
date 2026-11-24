//! The lane's ordering, which is the part that must not move: the run's
//! narrowing before the mode, the mode before the remembered grants, and the
//! grant only ever inside the asking arm.

use saya_agent::{ApprovalPolicy, SessionGrants};

use super::approval::Verdict;
use super::approval_tests::{Recording, query, tool, with_grants_and_tools};
use crate::agent::tools::consent::Answer;
use crate::agent::tools::shell_policy::ModeCell;

/// A grant is memory, not a mode: a connection an earlier session answer
/// recorded stays refused while the mode refuses it. The store is consulted
/// after the mode, never instead of it.
#[tokio::test]
async fn a_never_mode_refuses_a_granted_connection() {
    let operator = Recording::answering(Answer::Once);
    let grants = SessionGrants::default();
    grants.grant("sql:demo");
    let lane = with_grants_and_tools(
        ApprovalPolicy::Never,
        operator.clone(),
        saya_config::ResolvedTools::default(),
        grants,
    );
    let query_tool = tool("bounded_sql_query");

    assert_eq!(
        lane.decide(&query_tool, &query("demo")),
        Verdict::Refuse,
        "the mode refuses before the grant is read"
    );
    assert!(
        !lane.approve(&query_tool, &query("demo")).await,
        "a remembered grant cannot widen the mode"
    );
    assert_eq!(operator.calls(), 0, "a refused call is put to nobody");
}

/// The run's `[tools]` narrowing is consulted before the mode: a plugin name
/// the list took away is refused under `bypass`, where the mode alone would run
/// it. The bounded SQL family is not the list's to remove — `saya-config`
/// refuses a list that names one — so a stated list leaves it admitted.
#[tokio::test]
async fn the_tools_narrowing_is_consulted_before_the_mode() {
    let keep_read_only =
        saya_config::ResolvedTools::from_stated(Some(&["read".to_owned()])).expect("a valid list");
    let operator = Recording::answering(Answer::Once);
    let lane = with_grants_and_tools(
        ApprovalPolicy::Bypass,
        operator.clone(),
        keep_read_only,
        SessionGrants::default(),
    );
    let narrowing = serde_json::json!({});

    assert_eq!(
        lane.decide(&tool("request_clarification"), &narrowing),
        Verdict::Refuse,
        "the narrowed-away name is refused before the mode runs"
    );
    assert!(
        !lane
            .approve(&tool("request_clarification"), &narrowing)
            .await,
        "a narrowed-away plugin name is refused in bypass"
    );
    assert_eq!(
        operator.calls(),
        0,
        "nothing is asked about a narrowed name"
    );
    assert_eq!(
        lane.decide(&tool("bounded_sql_query"), &query("demo")),
        Verdict::Allow,
        "the list cannot narrow the connection-tied SQL family"
    );
}

/// A lane with nobody to answer claims no ask: the hook gate reads the same
/// table, and a question nobody can be shown is a refusal, never an ask.
#[test]
fn a_lane_with_nobody_to_answer_announces_no_ask() {
    let lane = super::approval::DbApprovalLane::new(
        ModeCell::new(ApprovalPolicy::Ask),
        SessionGrants::default(),
        None,
        saya_config::ResolvedTools::default(),
        None,
    );
    assert!(
        !lane.would_ask(&tool("bounded_sql_query"), &query("demo")),
        "no operator, no ask to pre-empt"
    );
}
