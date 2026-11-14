//! What a helper's toolset advertises, and the subject line it asks under.

use saya_agent::{ApprovalPolicy, ToolExecutor};
use saya_types::{AgentId, JobId};
use serde_json::json;

use super::super::child_toolset;
use super::super::tests::{Sandbox, checkout, implementer_view};
use super::helper_shell;
use super::support::{HELPER, HELPER_LINE, JOB};
use crate::agent::tools::consent::Caller;
use crate::agent::tools::shell_policy::{self, line};

/// The subject line a helper's shell asks under names the helper and its job,
/// and the root's own line is unchanged.
#[test]
fn the_helper_subject_line_names_the_helper_and_its_job() {
    let helper = Caller::Helper {
        agent_id: AgentId::parse(HELPER).unwrap(),
        job_id: JobId::parse(JOB).unwrap(),
    };
    assert_eq!(line(&helper), HELPER_LINE);
    let root = Caller::Root {
        job_id: JobId::parse(JOB).unwrap(),
    };
    assert_eq!(line(&root), "Run a shell command for job job-l5");
}

/// A helper's toolset advertises `shell` when the composition root attached
/// one, and dispatches the call — under the model's `bash` spelling too. With
/// no shell attached the name stays unadvertised and unsupported.
#[tokio::test]
async fn a_helper_toolset_advertises_shell_and_dispatches_it() {
    let checkout_id = checkout("helper-shell");
    let view = implementer_view(&checkout_id);
    let sandbox = Sandbox::new("helper-shell");
    let pair = sandbox.pair(&checkout_id);
    let (mut definitions, mut tools) = child_toolset(&view, &pair).unwrap();
    assert!(
        !definitions.iter().any(|tool| tool.name == "shell"),
        "no shell is advertised until one is attached"
    );

    let Some(shell) = helper_shell(&sandbox, ApprovalPolicy::Ask, None) else {
        return;
    };
    definitions.extend(tools.attach_shell(shell));
    assert!(definitions.iter().any(|tool| tool.name == "shell"));
    assert!(
        !definitions.iter().any(|tool| tool.name == "bash"),
        "`bash` is the model's spelling of `shell`, never a definition of its own"
    );

    // Where the host proves no sandbox (Linux today), the composed shell is
    // host-only: a call naming the sandbox is a host call, and with no
    // operator it is refused with the documented message rather than run
    // unconfined. Both spellings take the same lane.
    let sandbox_proven = shell_policy::prove(&sandbox.ws_root(), ApprovalPolicy::Ask).is_some();
    for name in ["shell", "bash"] {
        let outcome = tools
            .execute(name, json!({ "command": "true", "sandbox": true }))
            .await;
        if sandbox_proven {
            let ran = outcome.unwrap_or_else(|error| {
                panic!("{name} is dispatched to the helper's shell: {error}")
            });
            assert_eq!(ran["exit_code"], 0, "{ran}");
            assert_eq!(ran["sandboxed"], true, "{ran}");
        } else {
            let error = outcome.expect_err("a host-only shell cannot honour `sandbox: true`");
            assert!(
                error.to_string().contains("no operator is available"),
                "{name} is refused as an unapprovable host call: {error}"
            );
        }
    }
}

/// A helper's toolset advertises the web lane when the composition root
/// attached one, and dispatches its calls — both of them, because a helper's
/// search is the root's search. With no lane attached — the mode that reaches
/// no network — the names stay unadvertised and unsupported, so a helper
/// cannot reach the wire through a tool it was never given.
#[tokio::test]
async fn a_helper_toolset_advertises_the_web_lane_and_dispatches_it() {
    use crate::agent::tools::web::fixture as web;
    use std::sync::Arc;
    let checkout_id = checkout("helper-web-lane");
    let view = implementer_view(&checkout_id);
    let sandbox = Sandbox::new("helper-web-lane");
    let pair = sandbox.pair(&checkout_id);
    let (mut definitions, mut tools) = child_toolset(&view, &pair).unwrap();
    assert!(
        !definitions.iter().any(|tool| tool.name == "web_fetch"),
        "no web tool is advertised until a lane is attached"
    );
    assert!(
        matches!(
            tools.execute("web_fetch", web::call(&web::url("/a"))).await,
            Err(saya_agent::ToolError::UnsupportedTool)
        ),
        "a helper with no lane attached has no way to reach the wire"
    );

    let net = Arc::new(web::CannedNet::serving("a page").typed("text/plain"));
    let lane = web::web_tools(
        net.clone(),
        Some(web::searxng_backend()),
        ApprovalPolicy::Bypass,
        None,
    )
    .expect("bypass composes a lane");
    definitions.extend(tools.attach_web(Arc::new(lane)));
    for name in ["web_fetch", "web_search"] {
        assert!(
            definitions.iter().any(|tool| tool.name == name),
            "the helper advertises {name}"
        );
    }

    // The lane is attached, but the lean default still withholds the web pair
    // until the request asks for it: a helper that was not asked to fetch has
    // no web tool to reach for.
    assert!(
        matches!(
            tools.execute("web_fetch", web::call(&web::url("/a"))).await,
            Err(saya_agent::ToolError::UnsupportedTool)
        ),
        "the attached lane is not advertised without evidence"
    );
    tools.apply_evidence(crate::agent::tools::advertisement::Evidence::new(
        "read https://docs.example.org/a",
        true,
    ));

    let fetched = tools
        .execute("web_fetch", web::call(&web::url("/a")))
        .await
        .unwrap_or_else(|error| panic!("the call is dispatched to the helper's lane: {error}"));
    assert_eq!(fetched["status"], 200, "{fetched}");
    assert!(fetched["content"].as_str().unwrap().contains("a page"));

    // The wire serves the same bytes to both tools, and a page is not a
    // search answer: the point here is only that the call *reached* the lane
    // rather than being refused as a name this helper does not serve.
    let searched = tools
        .execute("web_search", serde_json::json!({"query": "rust"}))
        .await
        .expect_err("a page is not a JSON search answer");
    assert!(
        !matches!(searched, saya_agent::ToolError::UnsupportedTool),
        "the search is dispatched to the helper's lane: {searched}"
    );
}
