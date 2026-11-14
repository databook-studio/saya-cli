use super::*;
use crate::engine::episode::EpisodeRequest;

use saya_agent::ToolDefinition;
use saya_types::{Capabilities, Destination, FetchScope, StepSpec};

fn request() -> EpisodeRequest {
    EpisodeRequest {
        model: "mock-model".into(),
        profile_names: Vec::new(),
        memory_allows_candidate_writes: false,
    }
}

fn spec(capabilities: &Capabilities) -> StepSpec {
    StepSpec::new("the step", capabilities.clone(), None, Vec::new(), None).unwrap()
}

/// The explicit scope→permit mapping: a step approved any write-shaped
/// scope — workspace-write, fetch — carries the write
/// permit, because `LocalStateEffect` has one write-shaped variant and
/// the loop gates it on this one permit. What the step can actually
/// write stays bounded by construction: a scope the step lacks leaves
/// the tool absent from its definitions, so the union cannot admit a
/// tool the step never saw.
#[test]
fn every_write_shaped_scope_maps_onto_the_write_permit() {
    let mut caps = Capabilities::default();
    assert!(
        !limits(&request(), &spec(&caps)).permit_workspace_writes,
        "no write-shaped scope approved, no permit"
    );

    caps.workspace_write = true;
    assert!(limits(&request(), &spec(&caps)).permit_workspace_writes);

    // The mapping is the mapping: approving the scope carries the permit,
    // so wiring a tool needs no further limit change.
    let mut fetch = Capabilities::default();
    fetch.fetch = Some(
        FetchScope::new(vec![
            Destination::new("https", "example.com").expect("shaped"),
        ])
        .expect("shaped"),
    );
    assert!(
        limits(&request(), &spec(&fetch)).permit_workspace_writes,
        "a write-shaped scope must carry the write permit: {fetch:?}"
    );
}

/// The egress permit's half of the mapping: a step that approved a
/// plan-gated egress scope — fetch — carries `permit_external_effects`,
/// because the loop's misconfiguration guard would otherwise deny the fetch
/// tools in both
/// paths regardless of the scope. A step that approved no egress scope
/// (workspace-write alone included) does not carry it, and the default
/// keeps it off, so interactive turns are untouched. The union cannot
/// admit an external tool the step never saw: the definitions were
/// built from the same capabilities.
#[test]
fn an_egress_scope_maps_onto_the_external_effects_permit() {
    let mut default_caps = Capabilities::default();
    assert!(
        !limits(&request(), &spec(&default_caps)).permit_external_effects,
        "no egress scope approved, no permit"
    );

    // workspace-write alone is a write scope, not an egress scope.
    default_caps.workspace_write = true;
    assert!(!limits(&request(), &spec(&default_caps)).permit_external_effects);

    let mut fetch = Capabilities::default();
    fetch.fetch = Some(
        FetchScope::new(vec![
            Destination::new("https", "example.com").expect("shaped"),
        ])
        .expect("shaped"),
    );
    assert!(
        limits(&request(), &spec(&fetch)).permit_external_effects,
        "an approved fetch scope carries the egress permit"
    );
    assert!(
        limits(&request(), &spec(&fetch)).permit_workspace_writes,
        "fetch carries the write permit too: http_download is its write-shaped member"
    );
}

/// The definitions filter asks the same question the permit mapping does:
/// a write-shaped tool survives for a step that approved any write-shaped
/// scope, and a step that approved none must not see it.
#[test]
fn the_write_shaped_filter_keeps_what_a_write_shaped_scope_approved() {
    use saya_agent::{LocalStateEffect, ToolConcurrency, ToolEffect};

    let definition = ToolDefinition {
        name: "write_shaped_probe".to_owned(),
        description: "a probe tool".to_owned(),
        read_only: false,
        parameters: serde_json::json!({ "type": "object" }),
        effect: ToolEffect {
            database_data: false,
            external_side_effect: false,
            requires_approval: false,
            local_state: LocalStateEffect::WriteWorkspace,
        },
        concurrency: ToolConcurrency::Serial,
        completion: None,
    };
    let names = |universe: &[ToolDefinition]| {
        universe
            .iter()
            .map(|tool| tool.name.clone())
            .collect::<Vec<_>>()
    };

    assert!(
        definitions(std::slice::from_ref(&definition), &Capabilities::default()).is_empty(),
        "a step that approved no write-shaped scope must not see it"
    );

    let mut write = Capabilities::default();
    write.workspace_write = true;
    assert_eq!(
        names(&definitions(&[definition], &write)),
        vec!["write_shaped_probe"],
        "a workspace-write-only step carrying the permit passes the \
         class filter; which tool it sees was the builder's decision"
    );
}
