//! The coding root's `[tools] allow` dispatch gate: the root's advertisement
//! is its gate, so a name the job's list removed must be refused at the door.
#![cfg(any(target_os = "macos", target_os = "linux"))]

use std::sync::Arc;

use saya_agent::{ApprovalPolicy, CancellationToken, ToolError, ToolExecutor};

use super::direct::RootTools;
use super::direct_tests::{working_checkout_root, working_checkout_root_with};
use crate::agent::tools::child::tests_root_compose_fixture as fixture;

/// The root's dispatch door reads the narrowed advertisement: a name the job's
/// list removed is refused as unknown through both entry points, not merely
/// left unadvertised. The narrowing applied here is the one `RootTools::build`
/// applies to the same `defs` (`narrow(&mut direct.defs, job.tools())`).
#[tokio::test]
async fn the_root_refuses_a_name_the_list_removed() {
    let fixture = fixture::Fixture::new("direct-allow-refusal", &["tracked.txt"]).await;
    let mut root = working_checkout_root(&fixture).await;
    let stated =
        saya_config::ResolvedTools::from_stated(Some(&["read".to_string()])).expect("a valid list");
    {
        let RootTools::WorkingCheckout(direct) = &mut root else {
            panic!("the fixture composes a working checkout");
        };
        crate::agent::tools::narrow(&mut direct.defs, &stated);
        assert!(direct.advertises("read"), "the list keeps `read`");
        assert!(!direct.advertises("list"), "the list removes `list`");
    }

    let arguments = serde_json::json!({});
    let context = saya_agent::ToolExecutionContext {
        result_cap: 4096,
        cancellation: CancellationToken::new(),
    };
    for (door, refused) in [
        ("execute", root.execute("list", arguments.clone()).await),
        (
            "execute_with_context",
            root.execute_with_context("list", arguments, context).await,
        ),
    ] {
        let refused = refused.expect_err("a removed name must refuse, not be served");
        assert!(
            matches!(refused, ToolError::UnsupportedTool),
            "{door} must refuse a name the list removed: {refused:?}"
        );
        assert_eq!(
            refused.to_string(),
            "unsupported read-only tool",
            "{door} refuses with the same wording as an uncomposed member"
        );
    }
}

/// A real MCP lane for the root: the client's own shell fixture, started
/// through the client's own connect path.
fn mcp_lane(label: &str) -> Arc<crate::mcp::client::McpTools> {
    let fixture = crate::mcp::client::support::Fixture::new(label);
    let servers = fixture.servers(&[], crate::mcp::client::support::TIMEOUTS);
    let resolver = fixture.resolver(&[]);
    crate::mcp::client::McpTools::connect(
        &servers,
        &resolver,
        ApprovalPolicy::Bypass,
        None,
        &crate::agent::tools::consent::Caller::Root {
            job_id: saya_types::JobId::parse("job-allow-mcp").expect("a well-formed job id"),
        },
        CancellationToken::new(),
    )
    .expect("a configured server composes a lane")
}

/// `[tools] allow` governs the root's MCP tools exactly as it governs its own:
/// the server's whole entry keeps every tool of it, an exact name keeps that
/// one, and a list that names neither removes the family from the
/// advertisement *and* from the door. The names are third-party, so the
/// catalog governs them by shape — but they are governed, and a root that
/// advertised one the list could not remove would be the drift the catalog
/// test exists to catch.
#[tokio::test]
async fn the_root_governs_mcp_names_by_the_same_list() {
    let fixture = fixture::Fixture::new("direct-allow-mcp", &["tracked.txt"]).await;
    let mut root =
        working_checkout_root_with(&fixture, None, Some(mcp_lane("direct-allow-mcp"))).await;
    let stated = |words: &[&str]| {
        saya_config::ResolvedTools::from_stated(Some(
            &words
                .iter()
                .map(|word| (*word).to_string())
                .collect::<Vec<_>>(),
        ))
        .expect("a valid list")
    };
    {
        let RootTools::WorkingCheckout(direct) = &mut root else {
            panic!("the fixture composes a working checkout");
        };
        assert!(
            direct.advertises("mcp__fixture__echo"),
            "with no list the root advertises the server's tools"
        );
        crate::agent::tools::narrow(&mut direct.defs, &stated(&["read"]));
        assert!(
            !direct.advertises("mcp__fixture__echo"),
            "a list that names no server removes the family"
        );
    }
    let refused = root
        .execute("mcp__fixture__echo", serde_json::json!({"text": "hi"}))
        .await
        .expect_err("the list removed every MCP name");
    assert!(
        matches!(refused, ToolError::UnsupportedTool),
        "a removed MCP name is unknown at the door: {refused:?}"
    );

    // The server's whole entry keeps every tool of it, and an exact name keeps
    // exactly that one.
    let mut root =
        working_checkout_root_with(&fixture, None, Some(mcp_lane("direct-allow-mcp-whole"))).await;
    let RootTools::WorkingCheckout(direct) = &mut root else {
        panic!("the fixture composes a working checkout");
    };
    crate::agent::tools::narrow(&mut direct.defs, &stated(&["mcp__fixture", "read"]));
    assert!(direct.advertises("mcp__fixture__echo"));
    assert!(direct.advertises("mcp__fixture__secret"));
    crate::agent::tools::narrow(&mut direct.defs, &stated(&["mcp__fixture__echo", "read"]));
    assert!(
        !direct.advertises("mcp__fixture__secret"),
        "an exact name keeps that one and no sibling"
    );
}
