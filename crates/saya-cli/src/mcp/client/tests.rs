//! Unit tests for the MCP client: what a configured server contributes, what
//! the bounds drop, and how a call is consented.
//!
//! Every test here runs the real transport against the shell fixture in
//! [`super::fixture`] — a real child process, a real pipe, real JSON — because
//! the parts of this module worth pinning are exactly the ones a mock would
//! replace: that a listing arrives, that an oversized frame is discarded rather
//! than buffered, and that a server which stops answering is killed instead of
//! waited on.

use std::sync::Arc;
use std::time::Duration;

use saya_agent::{ApprovalPolicy, CancellationToken};
use saya_config::{MapSecretResolver, ResolvedMcpServers};

use super::McpTools;
use super::bounds::{MAX_TOOL_DESCRIPTION_CHARS, MAX_TOOL_SCHEMA_BYTES, MAX_TOOLS_PER_SERVER};
use crate::agent::tools::consent::{Caller, Operator};

#[path = "tests/call.rs"]
mod call;
#[path = "tests/consent.rs"]
mod consent;
use super::support::{Fixture, TIMEOUTS, lane};

/// No `[mcp_servers]` table, no server, no client: the default is off, and a
/// surface that composes nothing advertises nothing.
#[test]
fn no_configured_server_composes_no_lane() {
    let servers = ResolvedMcpServers::default();
    let resolver = MapSecretResolver::new(Vec::new());
    assert!(
        McpTools::connect(
            &servers,
            &resolver,
            ApprovalPolicy::Bypass,
            None,
            &Caller::Session,
            CancellationToken::new(),
        )
        .is_none()
    );
}

/// The two modes that promise the machine nothing compose no client at all, so
/// no `mcp__` tool reaches the model under them.
#[test]
fn the_no_server_modes_compose_no_lane() {
    let fixture = Fixture::new("modes");
    let servers = fixture.servers(&[], TIMEOUTS);
    for policy in [ApprovalPolicy::ReadOnly, ApprovalPolicy::Never] {
        assert!(
            lane(&servers, &fixture.resolver(&[]), policy, None).is_none(),
            "{policy:?} must compose no lane"
        );
    }
}

/// A configured server contributes its tools under its own namespace, with the
/// effect that says a call reaches outside the workspace.
#[test]
fn a_configured_server_contributes_its_tools() {
    let fixture = Fixture::new("listing");
    let servers = fixture.servers(&[], TIMEOUTS);
    let lane = lane(
        &servers,
        &fixture.resolver(&[]),
        ApprovalPolicy::Bypass,
        None,
    )
    .expect("a configured server composes a lane");
    let definitions = lane.definitions();
    let names: Vec<&str> = definitions.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "mcp__fixture__echo",
            "mcp__fixture__fail",
            "mcp__fixture__secret"
        ]
    );
    let echo = &definitions[0];
    assert!(
        echo.description.starts_with("[MCP server \"fixture\"] "),
        "the description must name where it came from: {}",
        echo.description
    );
    assert!(!echo.read_only, "a server's tool is never read-only here");
    assert!(echo.effect.external_side_effect);
    assert!(echo.effect.requires_approval);
    assert_eq!(echo.parameters["type"], "object");
    assert!(lane.reports().is_empty(), "{:?}", lane.reports());
}

/// A tool whose name a call cannot carry is dropped, and the drop is reported:
/// one unusable entry must not cost the server the tools beside it.
#[test]
fn a_tool_with_an_unusable_name_is_dropped_and_reported() {
    let fixture = Fixture::new("bad-names");
    let servers = fixture.servers(&[("FIXTURE_MODE", "bad_names")], TIMEOUTS);
    let lane = lane(
        &servers,
        &fixture.resolver(&[("FIXTURE_MODE", "bad_names")]),
        ApprovalPolicy::Bypass,
        None,
    )
    .expect("a lane is composed even when a tool is dropped");
    let names: Vec<String> = lane.definitions().iter().map(|d| d.name.clone()).collect();
    assert_eq!(names, ["mcp__fixture__ok_name"]);
    let reports = lane.reports().join("\n");
    assert!(reports.contains("has space"), "{reports}");
}

/// A description past the character cap is cut, not dropped, and says so: the
/// model reads a bounded description with a marker where the rest was.
#[test]
fn an_oversized_description_is_bounded() {
    let fixture = Fixture::new("big-description");
    fixture.oversized_description();
    let servers = fixture.servers(&[("FIXTURE_MODE", "big_description")], TIMEOUTS);
    let lane = lane(
        &servers,
        &fixture.resolver(&[("FIXTURE_MODE", "big_description")]),
        ApprovalPolicy::Bypass,
        None,
    )
    .expect("a lane is composed");
    let definition = &lane.definitions()[0];
    assert!(
        definition.description.chars().count() <= MAX_TOOL_DESCRIPTION_CHARS,
        "{} chars",
        definition.description.chars().count()
    );
    assert!(definition.description.ends_with('…'));
    assert!(
        definition
            .description
            .starts_with("[MCP server \"fixture\"] ")
    );
}

/// A schema past the byte cap is dropped rather than truncated: half a schema
/// is a contract the server never stated.
#[test]
fn an_oversized_schema_is_dropped() {
    let fixture = Fixture::new("big-schema");
    fixture.oversized_schema();
    let servers = fixture.servers(&[("FIXTURE_MODE", "big_schema")], TIMEOUTS);
    let lane = lane(
        &servers,
        &fixture.resolver(&[("FIXTURE_MODE", "big_schema")]),
        ApprovalPolicy::Bypass,
        None,
    )
    .expect("a lane is composed");
    assert!(lane.definitions().is_empty());
    let reports = lane.reports().join("\n");
    assert!(
        reports.contains("input schema") && reports.contains(&MAX_TOOL_SCHEMA_BYTES.to_string()),
        "{reports}"
    );
}

/// A server that advertises more tools than the cap contributes the cap, and
/// the cut is reported rather than passed off as the whole listing.
#[test]
fn a_tool_count_over_the_cap_is_cut_and_reported() {
    let fixture = Fixture::new("many-tools");
    let servers = fixture.servers(&[("FIXTURE_MODE", "many_tools")], TIMEOUTS);
    let lane = lane(
        &servers,
        &fixture.resolver(&[("FIXTURE_MODE", "many_tools")]),
        ApprovalPolicy::Bypass,
        None,
    )
    .expect("a lane is composed");
    assert_eq!(lane.definitions().len(), MAX_TOOLS_PER_SERVER);
    let reports = lane.reports().join("\n");
    assert!(
        reports.contains(&MAX_TOOLS_PER_SERVER.to_string()),
        "{reports}"
    );
}

/// A listing that arrives in pages is followed to its end.
#[test]
fn a_paged_listing_is_followed() {
    let fixture = Fixture::new("paged");
    let servers = fixture.servers(&[("FIXTURE_MODE", "paginate")], TIMEOUTS);
    let lane = lane(
        &servers,
        &fixture.resolver(&[("FIXTURE_MODE", "paginate")]),
        ApprovalPolicy::Bypass,
        None,
    )
    .expect("a lane is composed");
    let names: Vec<String> = lane.definitions().iter().map(|d| d.name.clone()).collect();
    assert!(
        names.contains(&"mcp__fixture__first".to_owned()),
        "{names:?}"
    );
    assert!(
        names.contains(&"mcp__fixture__echo".to_owned()),
        "{names:?}"
    );
}

/// A server that never answers is killed at its deadline, reported, and
/// contributes nothing — the surface is not left waiting on it.
#[test]
fn a_server_that_never_answers_is_reported() {
    let fixture = Fixture::new("hang");
    let servers = fixture.servers(&[("FIXTURE_MODE", "hang")], (300, TIMEOUTS.1));
    let started = std::time::Instant::now();
    let lane = lane(
        &servers,
        &fixture.resolver(&[("FIXTURE_MODE", "hang")]),
        ApprovalPolicy::Bypass,
        None,
    )
    .expect("a failing server does not remove the lane");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the deadline must end the wait: {:?}",
        started.elapsed()
    );
    assert!(lane.definitions().is_empty());
    let reports = lane.reports().join("\n");
    assert!(
        reports.contains("\"fixture\"") && reports.contains("did not answer"),
        "{reports}"
    );
}

/// A server that exits at once is reported by name, and its tools are absent.
#[test]
fn a_server_that_exits_at_once_is_reported() {
    let fixture = Fixture::new("exit");
    let servers = fixture.servers(&[("FIXTURE_MODE", "exit")], TIMEOUTS);
    let lane = lane(
        &servers,
        &fixture.resolver(&[("FIXTURE_MODE", "exit")]),
        ApprovalPolicy::Bypass,
        None,
    )
    .expect("a failing server does not remove the lane");
    assert!(lane.definitions().is_empty());
    let reports = lane.reports().join("\n");
    assert!(reports.contains("mcp server \"fixture\""), "{reports}");
}

/// A frame past the byte cap ends the exchange instead of being buffered: the
/// reader discards it as it arrives, so a server cannot make the client hold
/// an unbounded line.
#[test]
fn an_oversized_frame_is_refused() {
    let fixture = Fixture::new("huge-frame");
    let servers = fixture.servers(&[("FIXTURE_MODE", "huge_frame")], TIMEOUTS);
    let lane = lane(
        &servers,
        &fixture.resolver(&[("FIXTURE_MODE", "huge_frame")]),
        ApprovalPolicy::Bypass,
        None,
    )
    .expect("a failing server does not remove the lane");
    assert!(lane.definitions().is_empty());
    let reports = lane.reports().join("\n");
    assert!(reports.contains("longer than"), "{reports}");
}

/// A `mcp__` name belongs to this client and nothing else does, which is what
/// lets a surface route the call here without a second table.
#[test]
fn only_mcp_names_are_owned() {
    assert!(McpTools::owns("mcp__fixture__echo"));
    assert!(!McpTools::owns("shell"));
    assert!(!McpTools::owns("mcp__fixture"));
}

/// `ask` with a configured server composes a client, and one with no operator
/// still composes it: the refusal belongs to the call, not to the composition,
/// so the reason reaches the model instead of the tool quietly vanishing.
#[test]
fn ask_composes_a_lane_with_or_without_an_operator() {
    let fixture = Fixture::new("ask-composition");
    let servers = fixture.servers(&[], TIMEOUTS);
    let asked: Option<Arc<dyn Operator>> = Some(super::support::recording_operator(true));
    for operator in [None, asked] {
        let composed = lane(
            &servers,
            &fixture.resolver(&[]),
            ApprovalPolicy::Ask,
            operator,
        );
        let composed = composed.expect("ask with a configured server composes a lane");
        assert_eq!(composed.definitions().len(), 3);
    }
}
