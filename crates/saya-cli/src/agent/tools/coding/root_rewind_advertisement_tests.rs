//! The root's `rewind` is advertised from the first request. A provider caches
//! the tool list as a prefix, so a tool that joins the list after the first
//! checkpoint turns every later request into a cache miss. `rewind` can list an
//! empty checkpoint set truthfully, so it is offered from the start and never
//! joins late.
#![cfg(any(target_os = "macos", target_os = "linux"))]

use std::sync::Arc;

use super::direct_tools::DirectCodingTools;
use super::owner_denial::DenialTally;
use super::parent::CodingTools;
use super::root_rewind::RootRewind;
use crate::agent::tools::child::tests_root_compose_fixture as fixture;

async fn root_with_rewind(fixture: &fixture::Fixture) -> DirectCodingTools {
    let workspace = super::direct_tests::compose_root_data(
        fixture,
        saya_config::ResolvedTools::default(),
        None,
    )
    .await;
    let mut direct = DirectCodingTools::new(
        CodingTools {
            job: Arc::clone(&fixture.delegate_job),
            databases: None,
            denials: DenialTally::default(),
        },
        workspace.tools,
        workspace.definitions,
        workspace.approval,
        None,
        &saya_config::HelperAgents::default(),
        None,
        None,
        None,
        String::new(),
        saya_config::ResolvedTools::default(),
    );
    direct.attach_rewind(RootRewind::new(Arc::clone(&fixture.delegate_job)));
    direct
}

#[tokio::test]
async fn rewind_is_advertised_before_the_first_checkpoint() {
    let fixture = fixture::Fixture::new("root-rewind-first-request", &["tracked.txt"]).await;
    let direct = root_with_rewind(&fixture).await;
    assert!(
        direct.advertises("rewind"),
        "the tool list carries rewind from the first request, before any checkpoint exists"
    );
    let names: Vec<_> = direct
        .advertised_definitions()
        .into_iter()
        .map(|definition| definition.name)
        .collect();
    assert_eq!(
        names.last().map(String::as_str),
        Some("rewind"),
        "rewind is appended last, so the tools before it keep their positions"
    );
}
