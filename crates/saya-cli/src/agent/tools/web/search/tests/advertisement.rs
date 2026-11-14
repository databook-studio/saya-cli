use super::super::bounds::{
    DEFAULT_MAX_RESULTS, MAX_DOMAIN_BYTES, MAX_DOMAINS, MAX_QUERY_BYTES, MAX_RESULTS,
};
use super::super::{definition, is_web_search};
use crate::agent::tools::web::fixture as web;

/// The schema is one definition, and it tells the truth about the call: it
/// reaches a host outside the workspace, so it needs approval, and it changes
/// nothing of the agent's, so it is repeatable.
#[test]
fn the_definition_is_honest_about_its_effect() {
    let definition = definition();
    assert_eq!(definition.name, "web_search");
    assert!(is_web_search(&definition.name));
    assert!(!is_web_search("web_fetch"), "the names are not aliases");
    assert!(definition.read_only);
    assert!(definition.effect.external_side_effect);
    assert!(definition.effect.requires_approval);
    assert!(!definition.effect.database_data);
    assert_eq!(
        definition.effect.local_state,
        saya_agent::LocalStateEffect::None
    );

    let parameters = &definition.parameters;
    assert_eq!(parameters["required"], serde_json::json!(["query"]));
    assert_eq!(parameters["additionalProperties"], serde_json::json!(false));
    assert_eq!(
        parameters["properties"]["query"]["maxLength"],
        MAX_QUERY_BYTES
    );
    assert_eq!(
        parameters["properties"]["max_results"]["maximum"],
        MAX_RESULTS
    );
    assert_eq!(
        parameters["properties"]["max_results"]["default"],
        DEFAULT_MAX_RESULTS
    );
    for name in ["allowed_domains", "blocked_domains"] {
        assert_eq!(parameters["properties"][name]["maxItems"], MAX_DOMAINS);
        assert_eq!(
            parameters["properties"][name]["items"]["maxLength"],
            MAX_DOMAIN_BYTES
        );
    }
    assert!(
        !definition.description.contains("searxng") && !definition.description.contains("brave"),
        "the model reads what the tool does, not which backend the user configured: {}",
        definition.description
    );
}

/// A configured backend and a mode that reaches the network advertise the
/// tool; with no backend there is no tool at all — Saya runs no search
/// service of its own, so an unconfigured surface offers nothing to call.
#[test]
fn the_tool_exists_only_where_a_backend_is_configured() {
    let net = std::sync::Arc::new(web::CannedNet::serving("{}"));
    let configured = web::web_tools(
        net.clone(),
        Some(web::searxng_backend()),
        saya_agent::ApprovalPolicy::Bypass,
        None,
    )
    .expect("bypass composes a lane");
    assert!(
        configured
            .definitions()
            .iter()
            .any(|definition| definition.name == "web_search"),
        "a configured backend is advertised"
    );

    let unconfigured = web::web_tools(net, None, saya_agent::ApprovalPolicy::Bypass, None)
        .expect("bypass composes a lane");
    assert!(
        !unconfigured
            .definitions()
            .iter()
            .any(|definition| definition.name == "web_search"),
        "with no backend, nothing is advertised"
    );
    assert!(
        unconfigured
            .definitions()
            .iter()
            .any(|definition| definition.name == "web_fetch"),
        "and `web_fetch` is unaffected: the two settings are independent"
    );
}

/// The two modes that reach no host advertise nothing, whatever backend is
/// configured: `read-only` is the operator's statement that nothing leaves
/// this machine, and `never` reaches nothing at all.
#[test]
fn the_no_egress_modes_advertise_no_search() {
    for policy in [
        saya_agent::ApprovalPolicy::ReadOnly,
        saya_agent::ApprovalPolicy::Never,
    ] {
        let net = std::sync::Arc::new(web::CannedNet::serving("{}"));
        assert!(
            web::web_tools(net, Some(web::searxng_backend()), policy, None).is_none(),
            "{policy:?} composes no web lane at all"
        );
    }
}

/// `[tools] allow` can take the tool away: the list is applied after
/// composition, so a surface that composed a search tool loses it when the
/// list does not name it — and keeps it when it does.
#[test]
fn the_allow_list_can_take_the_search_tool_away() {
    let keep = saya_config::ResolvedTools {
        allow: Some(saya_config::ToolsAllow::parse(["web_search"]).expect("a governed name")),
    };
    let mut definitions = vec![definition()];
    crate::agent::tools::narrow(&mut definitions, &keep);
    assert_eq!(definitions.len(), 1, "a list naming it keeps it");

    let drop_it = saya_config::ResolvedTools {
        allow: Some(saya_config::ToolsAllow::parse(["web_fetch"]).expect("a governed name")),
    };
    let mut definitions = vec![definition()];
    crate::agent::tools::narrow(&mut definitions, &drop_it);
    assert!(
        definitions.is_empty(),
        "a list that does not name it removes it"
    );
}
