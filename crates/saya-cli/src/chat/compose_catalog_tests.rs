//! The session advertises the core's catalogue — one list, the one a job root
//! advertises — and none of the spellings the old session advertisement
//! carried. The tool catalogue is the core's; the session adds no second list.
//!
//! The old spellings (`http_fetch`, `http_download`, `run_command`) are names a
//! model can no longer be offered: the core's `web_fetch`, `web_search` and
//! `shell` are what a call reaches them by. A grant or an allow list that still
//! spells an old name is mapped at its own seam, not re-advertised here.
//!
//! One core name is a fact about the host rather than about the catalogue:
//! `shell` rides the list exactly where this host's sandbox proves for the
//! run's root. A non-interactive front cannot answer a host-lane call, so the
//! root states `HostLane::ProvenOnly` and a shell exists only over a proven
//! sandbox — proved on macOS, refused on Linux. Both rows ask the composition's
//! own probe rather than guessing from the platform.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use saya_agent::{ApprovalPolicy, NoopEventSink};

use super::ChatFront;
use super::compose::RunComposition;
use super::compose_fixture::{self, Composed, RootGuard};
use super::operator_front_fixture;
use crate::commands::coding::parent_turn_provider::{Reply, Scripted};

/// Load ceiling, not a product bound: how long one scripted turn may take.
const RUN: Duration = Duration::from_secs(120);

/// The names the old session advertisement carried, and which the run's own
/// catalogue has no definition for any more.
const OLD_SPELLINGS: [&str; 3] = ["http_fetch", "http_download", "run_command"];

/// The composed and opened run over one fixture, with the folder kept alive.
async fn compose_case(
    fixture: compose_fixture::Fixture,
    front: Arc<dyn ChatFront>,
    session: &'static str,
) -> (RunComposition, RootGuard) {
    let Composed {
        application,
        run,
        root,
    } = fixture.into_parts(front, session, ApprovalPolicy::Ask);
    let composition = RunComposition::compose(application, run)
        .await
        .expect("the session's run composes");
    composition.open().await.expect("the run opens");
    (composition, root)
}

/// The tool names one request advertises, in order.
fn advertised(body: &str) -> Vec<String> {
    let body: serde_json::Value = serde_json::from_str(body).expect("a request body is JSON");
    body["tools"]
        .as_array()
        .expect("a request advertises its tools")
        .iter()
        .filter_map(|tool| tool["function"]["name"].as_str().map(str::to_owned))
        .collect()
}

/// One turn on the composed run, waited for under the fixture's load ceiling.
async fn turn(composition: &RunComposition) {
    turn_asking(composition, "hello").await;
}

/// One turn on a request that shows database work: the database family is
/// advertised from the turn whose own words asked for it
/// (`plugins::database::family`).
async fn turn_about_data(composition: &RunComposition) {
    turn_asking(composition, "which tables hold the users?").await;
}

async fn turn_asking(composition: &RunComposition, request: &str) {
    let turn = tokio::time::timeout(
        RUN,
        composition.job().run_next_turn(&NoopEventSink, request),
    )
    .await
    .expect("the turn settles");
    assert!(turn.is_ok(), "the turn must run: {turn:?}");
}

/// The core names these rows pin, and `shell` exactly where this host's probe
/// proves: the same list either way but for that one host-decided member.
fn core_names(shell: bool) -> Vec<&'static str> {
    let mut names = vec!["read", "glob", "grep"];
    if shell {
        names.push("shell");
    }
    names
}

/// Whether this host's sandbox proves for the run's root, asked of the very
/// probe the root's shell composition asks (`shell_policy::prove`, over the
/// same working checkout and the mode this run states). The composition states
/// `HostLane::ProvenOnly` for a front that cannot approve, so it composes a
/// shell exactly where this answers `true`.
fn shell_proves(root: &Path) -> bool {
    crate::agent::tools::shell_policy::prove(root, ApprovalPolicy::Ask).is_some()
}

/// The session's first request advertises the core's names — `web_fetch`,
/// `web_search` and `shell` where this host proves one — and no old spelling.
#[tokio::test]
async fn a_sessions_first_request_advertises_the_cores_catalogue() {
    let provider = Scripted::start_with_script(vec![Reply::Text("hello")]);
    let fixture = compose_fixture::in_place_with_web_search("catalog", &provider.endpoint).await;
    let front = operator_front_fixture::non_interactive();
    let (composition, root) = compose_case(fixture, front.front(), "session-catalog").await;
    let shell = shell_proves(&root.checkout_root());
    turn(&composition).await;

    let bodies = provider.bodies();
    let offered = advertised(&bodies[0]);
    for name in ["web_fetch", "web_search"] {
        assert!(
            offered.iter().any(|advertised| advertised == name),
            "the core advertises {name}: {offered:?}"
        );
    }
    for name in core_names(shell) {
        assert!(
            offered.iter().any(|advertised| advertised == name),
            "the core advertises {name}: {offered:?}"
        );
    }
    assert_eq!(
        offered.iter().any(|advertised| advertised == "shell"),
        shell,
        "shell rides the catalogue exactly where this host's sandbox proves: {offered:?}"
    );
    for name in OLD_SPELLINGS {
        assert!(
            !offered.iter().any(|advertised| advertised == name),
            "the session's advertisement carries no {name}: {offered:?}"
        );
    }
}

/// The core's names come first and the plugin's database names after them: the
/// session appends the plugin's tools to the core's one list rather than
/// composing a list of its own.
#[tokio::test]
async fn the_plugins_database_tools_follow_the_cores_catalogue() {
    let provider = Scripted::start_with_script(vec![Reply::Text("hello")]);
    let fixture =
        compose_fixture::in_place_with_database("catalog-order", &provider.endpoint).await;
    let front = operator_front_fixture::non_interactive();
    let (composition, root) = compose_case(fixture, front.front(), "session-catalog-order").await;
    let shell = shell_proves(&root.checkout_root());
    turn_about_data(&composition).await;

    let bodies = provider.bodies();
    let offered = advertised(&bodies[0]);
    let position = |name: &str| {
        offered
            .iter()
            .position(|advertised| advertised == name)
            .unwrap_or_else(|| panic!("{name} is advertised: {offered:?}"))
    };
    for core in core_names(shell) {
        assert!(
            position(core) < position("bounded_sql_query"),
            "the core's {core} precedes the plugin's tools: {offered:?}"
        );
    }
    // And the old spellings are absent from this list too, database or not.
    for name in OLD_SPELLINGS {
        assert!(
            !offered.iter().any(|advertised| advertised == name),
            "no {name} is advertised: {offered:?}"
        );
    }
}

/// Both branches of the expectation the two rows read: a proving host's list
/// carries `shell`, a refusing host's does not, and nothing else moves with it.
#[test]
fn the_expected_core_names_carry_shell_exactly_where_the_probe_proves() {
    let proven = core_names(true);
    let refused = core_names(false);
    assert!(
        proven.contains(&"shell"),
        "a host whose sandbox proves is expected to advertise shell: {proven:?}"
    );
    assert!(
        !refused.contains(&"shell"),
        "a host whose sandbox refuses is expected to advertise no shell: {refused:?}"
    );
    let without_shell: Vec<&str> = proven
        .iter()
        .copied()
        .filter(|name| *name != "shell")
        .collect();
    assert_eq!(
        without_shell, refused,
        "the two branches differ in shell alone"
    );
}
