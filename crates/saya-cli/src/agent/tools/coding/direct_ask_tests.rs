//! The clarification ask on the root of a job with a working checkout: the
//! ask is no database tool, so a job that selected no profile still carries
//! it — and only when an operator surface can answer it.
#![cfg(any(target_os = "macos", target_os = "linux"))]

use std::sync::Arc;

use saya_agent::{ToolError, ToolExecutor};

use super::direct::RootTools;
use super::direct_tests::compose_root_data;
use super::direct_tools::DirectCodingTools;
use super::parent::CodingTools;
use crate::agent::tools::child::tests_root_compose_fixture as fixture;
use crate::commands::coding::database::CodingDatabaseTools;

/// The root under a stated data surface: `databases` is what a job with an
/// operator-attached ask, a selected profile, or neither composes. It rides
/// the compose request into the workspace toolset, which is what the root's
/// database names route to — the coordinator holds none on this variant.
async fn root_with(
    fixture: &fixture::Fixture,
    databases: Option<CodingDatabaseTools>,
) -> RootTools {
    let workspace =
        compose_root_data(fixture, saya_config::ResolvedTools::default(), databases).await;
    let coordinator = CodingTools {
        job: Arc::clone(&fixture.delegate_job),
        databases: None,
        denials: crate::agent::tools::coding::owner_denial::DenialTally::default(),
    };
    let decider = workspace.approval;
    RootTools::WorkingCheckout(Box::new(DirectCodingTools::new(
        coordinator,
        workspace.tools,
        workspace.definitions,
        decider,
        None,
        &saya_config::HelperAgents::default(),
        None,
        None,
        None,
        String::new(),
        saya_config::ResolvedTools::default(),
    )))
}

/// The job's data surface for a job that selected no profile: only the ask can
/// be on it, and only when an operator surface can answer.
fn ask_only(fixture: &fixture::Fixture, clarification: bool) -> CodingDatabaseTools {
    CodingDatabaseTools::new(
        fixture.runtime.clone(),
        fixture.store.clone(),
        std::collections::BTreeMap::new(),
        clarification,
    )
}

/// The coding root with **no** database profile advertises the ask when an
/// operator can answer it, and never advertises a SQL tool it cannot reach.
#[tokio::test]
async fn a_root_without_a_profile_advertises_the_ask_only_for_an_operator() {
    let fixture = fixture::Fixture::new("direct-ask", &["tracked.txt"]).await;
    let answered = root_with(&fixture, Some(ask_only(&fixture, true))).await;
    let advertised = answered.definitions();
    assert!(has(&advertised, "request_clarification"));
    assert!(!has(&advertised, "schema_discovery"));
    assert!(!has(&advertised, "bounded_sql_query"));
    // The root's own executor answers it: no connection, no profile.
    let result = answered
        .execute(
            "request_clarification",
            serde_json::json!({"question": "which grain?"}),
        )
        .await
        .expect("an ask needs no database connection");
    assert_eq!(result["asked"], serde_json::json!(true), "{result}");

    // A headless job has nobody to answer: the ask is not advertised, and the
    // name is unknown to the executor.
    let headless = root_with(&fixture, Some(ask_only(&fixture, false))).await;
    assert!(!has(&headless.definitions(), "request_clarification"));
    assert!(matches!(
        headless
            .execute(
                "request_clarification",
                serde_json::json!({"question": "which grain?"})
            )
            .await,
        Err(ToolError::UnsupportedTool)
    ));
}

fn has(defs: &[saya_agent::ToolDefinition], name: &str) -> bool {
    defs.iter().any(|d| d.name == name)
}
