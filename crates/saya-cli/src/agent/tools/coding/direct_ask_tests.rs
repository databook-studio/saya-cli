//! The clarification ask on the root of a job with a working checkout: the ask
//! is a database tool, so a job carries it only when it selected a profile —
//! and, like every other database name, only when an operator surface can
//! answer the calls it advertises.
#![cfg(any(target_os = "macos", target_os = "linux"))]

use std::sync::Arc;

use saya_agent::{ToolError, ToolExecutor};

use super::direct::RootTools;
use super::direct_tests::compose_root_data_with_profiles;
use super::direct_tools::DirectCodingTools;
use super::parent::CodingTools;
use crate::agent::tools::child::tests_database::sqlite_profile;
use crate::agent::tools::child::tests_root_compose_fixture as fixture;
use crate::commands::coding::database::CodingDatabaseTools;

/// The profile identity one fixture job selects: a SQLite profile this test
/// owns, derived under the fixture's own cache scope — the same derivation the
/// job's consent and the data surface use.
fn profile(fixture: &fixture::Fixture) -> saya_types::ProfileIdentity {
    let root = std::env::temp_dir().join(format!("saya-direct-ask-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let (profile, _path) = sqlite_profile(&root, "main");
    crate::profile_identity::profile_identity("main", &profile, &fixture.runtime.cache_scope)
}

/// The root under a stated data surface: `databases` is what a job with an
/// operator-attached ask, a selected profile, or neither composes. It rides
/// the compose request into the workspace toolset, which is what the root's
/// database names route to — the coordinator holds none on this variant.
async fn root_with(
    fixture: &fixture::Fixture,
    databases: Option<CodingDatabaseTools>,
) -> RootTools {
    let workspace = compose_root_data_with_profiles(
        fixture,
        saya_config::ResolvedTools::default(),
        databases,
        Vec::new(),
    )
    .await;
    root_over(fixture, workspace)
}

/// The same root over a job that selected the fixture profile.
async fn root_with_profile(
    fixture: &fixture::Fixture,
    databases: Option<CodingDatabaseTools>,
) -> RootTools {
    let workspace = compose_root_data_with_profiles(
        fixture,
        saya_config::ResolvedTools::default(),
        databases,
        vec![profile(fixture)],
    )
    .await;
    root_over(fixture, workspace)
}

fn root_over(
    fixture: &fixture::Fixture,
    workspace: crate::agent::tools::child::RootWorkspace,
) -> RootTools {
    let decider = workspace.approval;
    RootTools::WorkingCheckout(Box::new(DirectCodingTools::new(
        CodingTools {
            job: Arc::clone(&fixture.delegate_job),
            databases: None,
            denials: crate::agent::tools::coding::owner_denial::DenialTally::default(),
        },
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

/// The job's data surface for a job that selected no profile. `clarification`
/// is whether an operator surface is attached to answer an ask.
fn ask_only(fixture: &fixture::Fixture, clarification: bool) -> CodingDatabaseTools {
    CodingDatabaseTools::new(
        fixture.runtime.clone(),
        fixture.store.clone(),
        std::collections::BTreeMap::new(),
        clarification,
    )
}

/// The coding root with **no** database profile is offered no ask, whoever is
/// attached: a code-only job has no database question to put, so the name is
/// neither advertised nor known to the executor.
#[tokio::test]
async fn a_root_without_a_profile_is_offered_no_ask() {
    let fixture = fixture::Fixture::new("direct-ask-none", &["tracked.txt"]).await;
    for clarification in [true, false] {
        let root = root_with(&fixture, Some(ask_only(&fixture, clarification))).await;
        let advertised = root.definitions();
        assert!(
            !has(&advertised, "request_clarification"),
            "no profile means no database and no ask (operator: {clarification})"
        );
        assert!(matches!(
            root.execute(
                "request_clarification",
                serde_json::json!({"question": "which grain?"})
            )
            .await,
            Err(ToolError::UnsupportedTool)
        ));
    }
}

/// A job with a selected profile and an operator advertises the ask — placed
/// first, where the job's database names have always begun — and its executor
/// serves the call through the plugin lane.
#[tokio::test]
async fn a_root_with_a_profile_advertises_the_ask_when_an_operator_can_answer() {
    let fixture = fixture::Fixture::new("direct-ask", &["tracked.txt"]).await;
    let root = root_with_profile(&fixture, Some(selected_ask(&fixture, true))).await;
    let advertised = root.definitions();
    assert!(
        has(&advertised, "request_clarification"),
        "a job with a profile is offered the ask: {:?}",
        names(&advertised)
    );
    assert!(
        names(&advertised)
            .iter()
            .position(|name| name == "request_clarification")
            < names(&advertised)
                .iter()
                .position(|name| name == "schema_discovery"),
        "the ask keeps its place before the SQL tools: {:?}",
        names(&advertised)
    );
    let result = root
        .execute(
            "request_clarification",
            serde_json::json!({"question": "which grain?"}),
        )
        .await
        .expect("the plugin lane serves the ask");
    assert_eq!(result["asked"], serde_json::json!(true), "{result}");

    // The same job with nobody to answer offers no ask: an operator surface is
    // what makes the question usable.
    let headless = root_with_profile(&fixture, Some(selected_ask(&fixture, false))).await;
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

/// The job's data surface over the fixture profile: the shape a job that
/// selected a profile composes.
fn selected_ask(fixture: &fixture::Fixture, clarification: bool) -> CodingDatabaseTools {
    let mut selected = std::collections::BTreeMap::new();
    selected.insert("main".to_owned(), profile(fixture));
    CodingDatabaseTools::new(
        fixture.runtime.clone(),
        fixture.store.clone(),
        selected,
        clarification,
    )
}

fn names(defs: &[saya_agent::ToolDefinition]) -> Vec<String> {
    defs.iter()
        .map(|definition| definition.name.clone())
        .collect()
}

fn has(defs: &[saya_agent::ToolDefinition], name: &str) -> bool {
    defs.iter().any(|d| d.name == name)
}
