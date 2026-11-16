//! What `web_fetch` says it is, which modes advertise it, and that the one
//! `[tools] allow` filter can take it away.

use saya_agent::{ApprovalPolicy, LocalStateEffect};
use saya_config::is_governed_tool;

use super::super::advertised;
use super::super::bounds::max_body_bytes;
use super::super::definition::{NAME, definition, is_web_fetch};

/// The name the model calls is the name the schema carries, and the one table
/// a dispatch reads agrees with it.
#[test]
fn the_definition_is_the_name_the_model_calls() {
    let definition = definition();
    assert_eq!(definition.name, NAME);
    assert!(is_web_fetch(NAME));
    assert!(
        !is_web_fetch("http_fetch"),
        "the session's tool is not this one"
    );
}

/// The effect is the `shell`'s shape: the call reaches the world outside the
/// workspace, so it carries `external_side_effect` and `requires_approval`,
/// and it changes nothing here — which is what the model reads `read_only` for.
#[test]
fn the_effect_reaches_outside_without_changing_anything_here() {
    let effect = definition().effect;
    assert!(effect.external_side_effect);
    assert!(effect.requires_approval);
    assert!(!effect.database_data);
    assert_eq!(effect.local_state, LocalStateEffect::None);
    assert!(definition().read_only);
}

/// The schema states the bounds the tool enforces: a URL that fits the byte
/// ceiling, and a `max_bytes` whose ceiling is the lane's own pre-bound. What
/// the model reads is what the tool does.
#[test]
fn the_schema_states_the_bounds_the_tool_enforces() {
    let parameters = definition().parameters;
    let properties = &parameters["properties"];
    assert_eq!(properties["url"]["maxLength"], 2048);
    assert_eq!(parameters["required"][0], "url");
    assert_eq!(
        properties["max_bytes"]["maximum"].as_u64().unwrap() as usize,
        max_body_bytes(),
        "the advertised ceiling is the lane's own pre-bound, never a wider promise"
    );
    assert!(properties["max_bytes"]["default"].as_u64().unwrap() > 0);
}

/// `bypass` and `ask` reach the network, so they advertise the tool; the two
/// modes that reach no host advertise none at all.
#[test]
fn only_the_two_network_modes_advertise_the_tool() {
    assert!(advertised(ApprovalPolicy::Bypass));
    assert!(advertised(ApprovalPolicy::Ask));
    assert!(!advertised(ApprovalPolicy::ReadOnly));
    assert!(!advertised(ApprovalPolicy::Never));
}

/// The name is in the governed catalog, so `[tools] allow` may name it and a
/// list that omits it removes the advertisement — the same filter every other
/// governed capability passes through.
#[test]
fn the_allow_list_governs_the_tool() {
    assert!(is_governed_tool(NAME));
    let mut definitions = vec![definition()];
    let stated = ["read".to_string()];
    crate::agent::tools::narrow(
        &mut definitions,
        &saya_config::ResolvedTools::from_stated(Some(&stated)).unwrap(),
    );
    assert!(
        definitions.is_empty(),
        "a list that does not name web_fetch takes it away"
    );

    let mut kept = vec![definition()];
    let named = [NAME.to_string()];
    crate::agent::tools::narrow(
        &mut kept,
        &saya_config::ResolvedTools::from_stated(Some(&named)).unwrap(),
    );
    assert_eq!(kept.len(), 1, "a list that names it keeps it");
}
