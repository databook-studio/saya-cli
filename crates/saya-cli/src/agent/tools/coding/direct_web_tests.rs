//! The root's web lane: advertised only when the request gives it evidence —
//! an http(s) URL, or a configured search backend — and routed to the fetch
//! tool when it is.
#![cfg(any(target_os = "macos", target_os = "linux"))]

use std::sync::Arc;

use saya_agent::{ApprovalDecider, ApprovalPolicy, ToolDefinition, ToolError, ToolExecutor};

use super::direct_tests::{working_checkout_root, working_checkout_root_with_request};
use crate::agent::tools::child::tests_root_compose_fixture as fixture;

/// The root advertises `web_fetch` when its mode reaches the network and the
/// request names a URL, routes a call to the fetch tool, and advertises nothing
/// at all when it does not: a name the composition dropped is unsupported, the
/// same refusal the loop's advertisement implies.
#[tokio::test]
async fn the_root_advertises_web_fetch_and_routes_it() {
    use crate::agent::tools::web::fixture as web_fixture;
    let fixture = fixture::Fixture::new("direct-web-fetch", &["tracked.txt"]).await;
    let net = Arc::new(web_fixture::CannedNet::serving("a page").typed("text/plain"));
    let web =
        web_fixture::web_tools(net, None, ApprovalPolicy::Bypass, None).expect("bypass advertises");
    let quiet = working_checkout_root_with_request(
        &fixture,
        Some(Arc::new(web)),
        None,
        "inspect the repository".to_owned(),
        saya_config::ResolvedTools::default(),
    )
    .await;
    assert!(
        !has(&quiet.definitions(), "web_fetch"),
        "the lane is attached, but the request names no URL"
    );

    let net = Arc::new(web_fixture::CannedNet::serving("a page").typed("text/plain"));
    let web =
        web_fixture::web_tools(net, None, ApprovalPolicy::Bypass, None).expect("bypass advertises");
    let root = working_checkout_root_with_request(
        &fixture,
        Some(Arc::new(web)),
        None,
        "read https://docs.example.org/a".to_owned(),
        saya_config::ResolvedTools::default(),
    )
    .await;
    let definition = find(&root.definitions(), "web_fetch").clone();
    let call = web_fixture::call(&web_fixture::url("/a"));
    assert!(root.approve(&definition, &call).await);
    let fetched = root
        .execute("web_fetch", call)
        .await
        .expect("the fetch runs");
    assert_eq!(fetched["status"], 200, "{fetched}");

    let without = working_checkout_root(&fixture).await;
    assert!(!has(&without.definitions(), "web_fetch"));
    assert!(matches!(
        without
            .execute(
                "web_fetch",
                serde_json::json!({"url": "https://docs.example.org/a"})
            )
            .await,
        Err(ToolError::UnsupportedTool)
    ));
}

fn find<'a>(definitions: &'a [ToolDefinition], name: &str) -> &'a ToolDefinition {
    definitions
        .iter()
        .find(|definition| definition.name == name)
        .unwrap_or_else(|| panic!("{name} is advertised"))
}

fn has(defs: &[ToolDefinition], name: &str) -> bool {
    defs.iter().any(|d| d.name == name)
}
