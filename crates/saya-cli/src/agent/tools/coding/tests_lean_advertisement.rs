//! What the coding root advertises by default, and what the request itself
//! turns back on. The rule lives in `agent::tools::advertisement`; these tests
//! prove the root applies it to the list it serves.
#![cfg(any(target_os = "macos", target_os = "linux"))]

use std::sync::Arc;

use saya_agent::{ApprovalPolicy, ToolDefinition};
use saya_config::{ResolvedTools, WebSearchBackend};

use super::direct_tests::{working_checkout_root, working_checkout_root_with_request};
use crate::agent::tools::child::tests_root_compose_fixture as fixture;
use crate::agent::tools::web::fixture as web_fixture;

fn has(definitions: &[ToolDefinition], name: &str) -> bool {
    definitions.iter().any(|definition| definition.name == name)
}

#[tokio::test]
async fn the_root_advertises_the_lean_default_set() {
    let fixture = fixture::Fixture::new("lean-root-default", &["tracked.txt"]).await;
    let root = working_checkout_root(&fixture).await;
    let definitions = root.definitions();
    for name in ["read", "write", "edit", "glob", "grep", "checklist"] {
        assert!(
            has(&definitions, name),
            "{name} is advertised: {definitions:?}"
        );
    }
    for name in [
        "list",
        "delete",
        "helpers",
        "web_fetch",
        "web_search",
        "notebook_edit",
        "shell_output",
        "shell_kill",
    ] {
        assert!(!has(&definitions, name), "{name} is not advertised");
    }
}

#[tokio::test]
async fn a_url_in_the_request_advertises_web_fetch() {
    let fixture = fixture::Fixture::new("lean-root-url", &["tracked.txt"]).await;
    let net = Arc::new(web_fixture::CannedNet::serving("a page"));
    let web =
        web_fixture::web_tools(net, None, ApprovalPolicy::Bypass, None).expect("bypass advertises");
    let quiet = working_checkout_root_with_request(
        &fixture,
        Some(Arc::new(web)),
        None,
        "inspect the repository".to_owned(),
        ResolvedTools::default(),
    )
    .await;
    assert!(!has(&quiet.definitions(), "web_fetch"));

    let net = Arc::new(web_fixture::CannedNet::serving("a page"));
    let web =
        web_fixture::web_tools(net, None, ApprovalPolicy::Bypass, None).expect("bypass advertises");
    let asked = working_checkout_root_with_request(
        &fixture,
        Some(Arc::new(web)),
        None,
        "read https://docs.example.org/a".to_owned(),
        ResolvedTools::default(),
    )
    .await;
    assert!(has(&asked.definitions(), "web_fetch"));
}

#[tokio::test]
async fn a_configured_web_backend_advertises_the_web_pair_without_a_url() {
    let fixture = fixture::Fixture::new("lean-root-web-config", &["tracked.txt"]).await;
    let backend = WebSearchBackend::Searxng {
        endpoint: "https://search.example.org/search".to_owned(),
    };
    let net = Arc::new(web_fixture::CannedNet::serving("a page"));
    let web = web_fixture::web_tools(net, Some(backend), ApprovalPolicy::Bypass, None)
        .expect("bypass advertises");
    let root = working_checkout_root_with_request(
        &fixture,
        Some(Arc::new(web)),
        None,
        "inspect the repository".to_owned(),
        ResolvedTools::default(),
    )
    .await;
    let definitions = root.definitions();
    assert!(has(&definitions, "web_fetch"), "{definitions:?}");
    assert!(has(&definitions, "web_search"), "{definitions:?}");
}

#[tokio::test]
async fn a_request_that_asks_for_helpers_advertises_them() {
    let fixture = fixture::Fixture::new("lean-root-helpers", &["tracked.txt"]).await;
    let quiet = working_checkout_root(&fixture).await;
    assert!(!has(&quiet.definitions(), "helpers"));

    let asked = working_checkout_root_with_request(
        &fixture,
        None,
        None,
        "use two helpers to inspect this".to_owned(),
        ResolvedTools::default(),
    )
    .await;
    assert!(has(&asked.definitions(), "helpers"));
}

#[tokio::test]
async fn an_explicit_allow_entry_readvertises_list() {
    let fixture = fixture::Fixture::new("lean-root-allow-list", &["tracked.txt"]).await;
    let tools =
        ResolvedTools::from_stated(Some(&["list".to_owned()])).expect("list is a governed tool");
    let root = working_checkout_root_with_request(&fixture, None, None, String::new(), tools).await;
    let definitions = root.definitions();
    assert!(has(&definitions, "list"), "{definitions:?}");
    assert!(!has(&definitions, "delete"), "{definitions:?}");
}

#[tokio::test]
async fn a_stated_allow_list_still_narrows() {
    let fixture = fixture::Fixture::new("lean-root-allow-narrow", &["tracked.txt"]).await;
    let tools =
        ResolvedTools::from_stated(Some(&["read".to_owned()])).expect("read is a governed tool");
    let root = working_checkout_root_with_request(&fixture, None, None, String::new(), tools).await;
    let definitions = root.definitions();
    assert!(has(&definitions, "read"), "{definitions:?}");
    assert!(!has(&definitions, "edit"), "{definitions:?}");
    assert!(!has(&definitions, "list"), "{definitions:?}");
}
