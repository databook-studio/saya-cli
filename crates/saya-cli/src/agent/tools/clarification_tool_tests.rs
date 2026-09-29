//! Tests for the `request_clarification` surface (B3c): the definition the
//! model is advertised, and the executor arm that answers a landed ask with
//! the short result — without touching any connection, with or without data
//! sharing.

use super::database_tools::DatabaseTools;
use saya_agent::ToolExecutor;

fn valid_args() -> serde_json::Value {
    serde_json::json!({
        "question": "Which metric should \"active users\" use?",
        "options": ["sessions in the last 30 days", "purchases in the last 90 days"]
    })
}

/// The tool is advertised in every surface's definitions: it declares no
/// effect and no approval need (an ask touches nothing), is read-only-shaped,
/// and its schema states the bounds.
#[test]
fn request_clarification_is_advertised_with_an_effect_none_declaration() {
    for allow_query_data in [false, true] {
        let definitions = DatabaseTools::definitions(allow_query_data, false, false, false, false);
        let definition = definitions
            .iter()
            .find(|definition| definition.name == "request_clarification")
            .unwrap_or_else(|| {
                panic!(
                    "request_clarification must be advertised (allow_query_data: {allow_query_data})"
                )
            });
        assert!(definition.read_only);
        assert!(
            !definition.effect.database_data
                && !definition.effect.external_side_effect
                && !definition.effect.requires_approval,
            "an ask touches nothing and needs no approval: {:?}",
            definition.effect
        );
        assert!(
            definition.effect.local_state == saya_agent::LocalStateEffect::None,
            "the ask writes no local state: {:?}",
            definition.effect
        );
        assert_eq!(
            definition.parameters["required"],
            serde_json::json!(["question"]),
            "the question is the one required argument"
        );
        assert_eq!(
            definition.parameters["properties"]["options"]["maxItems"],
            serde_json::json!(6),
            "the schema states the option bound so the model cannot drift past it"
        );
    }
}

/// A landed ask executes through the executor arm without any connection: an
/// empty registry (no connector at all) still answers.
#[tokio::test]
async fn the_executor_answers_the_ask_without_touching_a_connection() {
    let tools = DatabaseTools::new(None, 10, false);
    let result = tools
        .execute("request_clarification", valid_args())
        .await
        .expect("an ask needs no database");
    assert_eq!(result["asked"], serde_json::json!(true), "{result}");
}

/// The ask is not data: it works when the data-sharing gate is closed, where
/// every query tool refuses.
#[tokio::test]
async fn the_ask_works_when_data_sharing_is_off() {
    let tools = DatabaseTools::new(None, 10, false);
    let result = tools
        .execute(
            "request_clarification",
            serde_json::json!({"question": "which one?"}),
        )
        .await
        .expect("the ask is not a query");
    assert_eq!(result["asked"], serde_json::json!(true), "{result}");
}

/// Unknown keys are still rejected — the arm rides the same argument
/// validation every other tool goes through.
#[tokio::test]
async fn unknown_keys_are_rejected() {
    let tools = DatabaseTools::new(None, 10, false);
    let error = tools
        .execute(
            "request_clarification",
            serde_json::json!({"question": "which one?", "bogus": true}),
        )
        .await
        .expect_err("an unsupported property is rejected");
    assert!(
        matches!(error, saya_agent::ToolError::UnsupportedProperty),
        "{error}"
    );
}
