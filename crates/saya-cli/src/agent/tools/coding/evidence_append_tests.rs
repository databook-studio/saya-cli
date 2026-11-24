//! The evidence-gated tools only ever append: the list a surface advertises
//! before the request asks for `web_fetch`, `web_search` or `helpers` is a
//! prefix of the list it advertises after.
//!
//! A provider caches a request's prefix, so a tool that joins the list in the
//! middle moves every name after it and turns the next request into a cache
//! miss. A gated tool therefore sits at the end of the composition, where
//! turning it on cannot move anything that was already advertised.
#![cfg(any(target_os = "macos", target_os = "linux"))]

use std::sync::Arc;

use saya_agent::ApprovalPolicy;

use super::direct::RootTools;
use super::direct_tests::working_checkout_root_with_request;
use crate::agent::tools::child::tests_root_compose_fixture as fixture;
use crate::agent::tools::web::fixture as web_fixture;

/// The names a root advertises, in composition order.
fn names(root: &RootTools) -> Vec<String> {
    root.definitions()
        .into_iter()
        .map(|definition| definition.name)
        .collect()
}

/// A root over `case` with a web lane attached, asked `request`.
async fn root_asked(case: &fixture::Fixture, request: &str) -> RootTools {
    let net = Arc::new(web_fixture::CannedNet::serving("a page"));
    let web = web_fixture::web_tools(net, None, ApprovalPolicy::Bypass, None)
        .expect("bypass advertises the web lane");
    working_checkout_root_with_request(
        case,
        Some(Arc::new(web)),
        None,
        request.to_owned(),
        saya_config::ResolvedTools::default(),
    )
    .await
}

/// Test 1 — the web pair and `helpers` join the list at its end: every name the
/// quiet request advertised keeps the position it had, and the gated names come
/// after all of them.
#[tokio::test]
async fn the_web_and_helper_gate_only_appends() {
    let case = fixture::Fixture::new("guard-append-gated", &["tracked.txt"]).await;
    let quiet = names(&root_asked(&case, "inspect the repository").await);
    let asked = names(
        &root_asked(
            &case,
            "delegate the checks to a helper at https://docs.example.org/a",
        )
        .await,
    );
    println!("quiet: {quiet:?}\nasked: {asked:?}");

    for name in ["web_fetch", "helpers"] {
        assert!(
            asked.contains(&name.to_owned()),
            "the request asks for {name}: {asked:?}"
        );
        assert!(
            !quiet.contains(&name.to_owned()),
            "{name} is gated, not default: {quiet:?}"
        );
    }
    assert_eq!(
        asked.get(..quiet.len()),
        Some(quiet.as_slice()),
        "the gated tools only append: every name the quiet request advertised keeps its position \
         (quiet {quiet:?}, asked {asked:?})"
    );
}

/// Test 2 — the same rule for `helpers` alone, so a tool appended after the web
/// pair cannot hide an insertion in the middle: the list grows at its end and
/// nowhere else.
#[tokio::test]
async fn the_helper_gate_alone_appends() {
    let case = fixture::Fixture::new("guard-append-helpers", &["tracked.txt"]).await;
    let quiet = names(&root_asked(&case, "inspect the repository").await);
    let asked = names(&root_asked(&case, "use two helpers to inspect this").await);
    assert!(
        asked.contains(&"helpers".to_owned()),
        "the request asks for a helper: {asked:?}"
    );
    assert_eq!(
        asked.get(..quiet.len()),
        Some(quiet.as_slice()),
        "`helpers` only appends (quiet {quiet:?}, asked {asked:?})"
    );
}
