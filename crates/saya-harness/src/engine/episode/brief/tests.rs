use super::*;
use crate::engine::episode::EpisodeRequest;

use saya_agent::ToolDefinition;
use saya_types::{Capabilities, Destination, FetchScope, RunnerScope, StepSpec};

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
/// scope — workspace-write, scratch, runner, fetch — carries the write
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

    // Scratch, runner, and fetch were refused at `--allow` parse time
    // until their wiring slices landed — fetch is wired now (S2) — but
    // the mapping is the mapping: approving the scope carries the
    // permit, so wiring a tool needs no further limit change.
    let mut scratch = Capabilities::default();
    scratch.scratch = true;
    let mut runner = Capabilities::default();
    runner.runner = Some(RunnerScope::new(vec!["python3".to_owned()]).expect("shaped"));
    let mut fetch = Capabilities::default();
    fetch.fetch = Some(
        FetchScope::new(vec![
            Destination::new("https", "example.com").expect("shaped"),
        ])
        .expect("shaped"),
    );
    for shape in [scratch, runner, fetch] {
        assert!(
            limits(&request(), &spec(&shape)).permit_workspace_writes,
            "a write-shaped scope must carry the write permit: {shape:?}"
        );
    }
}

/// The egress permit's half of the mapping: a step that approved a
/// plan-gated egress scope — fetch, or runner (a child may carry
/// `net_allow`) — carries `permit_external_effects`, because the loop's
/// misconfiguration guard would otherwise deny the fetch tools in both
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

    let mut runner = Capabilities::default();
    runner.runner = Some(RunnerScope::new(vec!["python3".to_owned()]).expect("shaped"));
    assert!(
        limits(&request(), &spec(&runner)).permit_external_effects,
        "a runner child may carry net_allow egress, so the mapping includes runner"
    );
}

/// The definitions filter asks the same question the permit mapping does:
/// a write-shaped tool survives for a step that approved any write-shaped
/// scope. Pinned with the real `scratch_sql` definition, because it is
/// the first write-shaped tool whose approval does not come from
/// `workspace_write` — a scratch-only step (the `--allow scratch` shape)
/// must see it, and a step that approved no write-shaped scope must not.
#[test]
fn the_write_shaped_filter_keeps_what_a_write_shaped_scope_approved() {
    use crate::scratch::{SCRATCH_SQL_TOOL, ScratchSql};

    let definition = ScratchSql::definition();
    let names = |universe: &[ToolDefinition]| {
        universe
            .iter()
            .map(|tool| tool.name.clone())
            .collect::<Vec<_>>()
    };

    let mut scratch = Capabilities::default();
    scratch.scratch = true;
    assert_eq!(
        names(&definitions(std::slice::from_ref(&definition), &scratch)),
        vec![SCRATCH_SQL_TOOL],
        "a scratch-only step must see scratch_sql — the filter is the \
         second lock, never a strip of an approved tool"
    );

    assert!(
        definitions(std::slice::from_ref(&definition), &Capabilities::default()).is_empty(),
        "a step that approved no write-shaped scope must not see it"
    );

    let mut workspace_write = Capabilities::default();
    workspace_write.workspace_write = true;
    assert_eq!(
        names(&definitions(&[definition], &workspace_write)),
        vec![SCRATCH_SQL_TOOL],
        "a workspace-write-only step carrying the permit passes the \
         class filter; which tool it sees was the builder's decision"
    );
}
