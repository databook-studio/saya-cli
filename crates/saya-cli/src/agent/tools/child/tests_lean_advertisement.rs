//! What a helper advertises by default, and what its own request turns back
//! on. The helper composes no `helpers` tool — only the root delegates — but it
//! applies the same evidence rule to `list`, `delete`, and the web pair.
#![cfg(any(target_os = "macos", target_os = "linux"))]

use std::sync::Arc;

use saya_agent::{ApprovalPolicy, CancellationToken, ToolDefinition};
use saya_config::ResolvedTools;
use saya_harness::coding::child::ChildFactory;

use super::CliChildFactory;
use super::tests::{Sandbox, checkout, implementer_view};
use super::tests_factory::{spec_with_prompt, test_ai};
use crate::agent::tools::web::fixture as web_fixture;

fn factory(sandbox: &Sandbox, checkout_id: &saya_types::ManagedCheckoutId) -> CliChildFactory {
    CliChildFactory::new(
        sandbox.pair(checkout_id),
        test_ai(),
        Arc::new(saya_config::MapSecretResolver::new([])),
    )
}

fn has(definitions: &[ToolDefinition], name: &str) -> bool {
    definitions.iter().any(|definition| definition.name == name)
}

#[test]
fn a_helper_advertises_the_lean_default_set() {
    let checkout_id = checkout("checkout");
    let view = implementer_view(&checkout_id);
    let sandbox = Sandbox::new("lean-helper-default");
    let components = factory(&sandbox, &checkout_id)
        .build(
            &spec_with_prompt(view, "inspect the assigned source"),
            &CancellationToken::new(),
        )
        .expect("the child composes");
    let definitions = components.definitions();
    for name in ["read", "write", "edit", "glob", "grep", "checklist"] {
        assert!(
            has(definitions, name),
            "{name} is advertised: {definitions:?}"
        );
    }
    for name in ["list", "delete", "helpers", "web_fetch", "web_search"] {
        assert!(!has(definitions, name), "{name} is not advertised");
    }
}

#[test]
fn a_url_in_the_helper_request_advertises_web_fetch() {
    let checkout_id = checkout("checkout");
    let view = implementer_view(&checkout_id);
    let sandbox = Sandbox::new("lean-helper-url");
    let net = Arc::new(web_fixture::CannedNet::serving("a page"));
    let web =
        web_fixture::web_tools(net, None, ApprovalPolicy::Bypass, None).expect("bypass advertises");
    let components = factory(&sandbox, &checkout_id)
        .with_web(Some(Arc::new(web)))
        .build(
            &spec_with_prompt(view, "read https://docs.example.org/a"),
            &CancellationToken::new(),
        )
        .expect("the child composes");
    assert!(has(components.definitions(), "web_fetch"));
}

#[test]
fn an_explicit_allow_entry_readvertises_list_for_a_helper() {
    let checkout_id = checkout("checkout");
    let view = implementer_view(&checkout_id);
    let sandbox = Sandbox::new("lean-helper-allow-list");
    let tools =
        ResolvedTools::from_stated(Some(&["list".to_owned()])).expect("list is a governed tool");
    let components = factory(&sandbox, &checkout_id)
        .with_tools(tools)
        .build(
            &spec_with_prompt(view, "inspect the assigned source"),
            &CancellationToken::new(),
        )
        .expect("the child composes");
    assert!(has(components.definitions(), "list"));
}
