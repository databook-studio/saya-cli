//! Twins of
//! `interactive/session/universe/tests/grant_recompute_and_host_refusal.rs` on
//! the live chat path: what a held grant reaches, what a grant recorded
//! mid-session is honoured for on the next turn, and what the live catalogue
//! does with a name it no longer carries.
//!
//! The old tests read the session's own `definitions(…)` and its
//! `TerminalApproval` decider. The live run's mode surface reads
//! `job.launch_grants()` (the launch's seed store, `--allow`), while the
//! per-call deciders — the write ask and the shell's prompt — read the
//! operator's own store, the one a session-wide answer is recorded into. Both
//! facts are pinned here: a grant recorded mid-session is honoured on the next
//! turn's *decision*, and the advertisement is the launch's.

use std::sync::Arc;

use saya_agent::ApprovalPolicy;
use serde_json::json;

use super::ChatFront;
use super::compose_fixture;
use super::safety_twin_fixture::{
    Fixed, HOST_MARKER, WORKSPACE_WRITE, advertised, assert_no_host_route, compose, offers,
    refused_as_unknown, result_of, turn,
};
use crate::agent::tools::consent::Answer;
use crate::commands::coding::parent_turn_provider::{Reply, Scripted};

/// Every family's grant at once, including the SQL ride-along the old row
/// pinned as widening nothing of its own.
const EVERY_FAMILY: [&str; 4] = [
    "workspace-write",
    "fetch:https+docs.example",
    "command:cargo",
    "sql:docs",
];

/// The names the old posture hid, under the live catalogue's spelling: the two
/// fetch doors and the old command name have no definition any more, and the
/// write family's names and the shell are what a call would reach them by.
const HIDDEN: [&str; 6] = [
    "write",
    "edit",
    "web_fetch",
    "http_fetch",
    "http_download",
    "run_command",
];

/// Every family grant at once cannot widen a restrictive posture: `read-only`
/// and `never` never consult grants, so every family stays hidden — asserted
/// with a front that *could* answer an ask, so nothing is hidden merely because
/// the surface has nobody to put a call to.
///
/// The shell half is a fact about the host, so it is stated the way the
/// catalogue rows state it, with the `shell_policy::prove` the composition
/// itself asks: `never` reaches no shell on any host, and `read-only` keeps the
/// sandbox lane alone, so the host-routed `touch` this row drives runs
/// write-denied there rather than writing a marker on the host. Never weakened:
/// no family grant makes a host command reachable.
#[tokio::test]
async fn no_family_grant_widens_a_restrictive_posture() {
    for (index, mode) in [ApprovalPolicy::ReadOnly, ApprovalPolicy::Never]
        .into_iter()
        .enumerate()
    {
        let case = format!("twin-grants-restrictive-{index}");
        let provider = Scripted::start_with_script(vec![
            Reply::Call {
                name: "shell",
                arguments: json!({"command": format!("touch {HOST_MARKER}")}),
            },
            Reply::Text("hello"),
        ]);
        let fixture = compose_fixture::in_place_with_web_search(&case, &provider.endpoint).await;
        let front = Fixed::new(Answer::Once);
        let (composition, root) = compose(
            fixture,
            Arc::clone(&front) as Arc<dyn ChatFront>,
            "twin-grants-restrictive",
            mode,
            &EVERY_FAMILY,
        )
        .await;

        let _ = turn(&composition, "hello").await;

        let bodies = provider.bodies();
        let offered = advertised(&bodies[0]);
        for tool in HIDDEN {
            assert!(
                !offers(&offered, tool),
                "{mode:?} stays closed under every family grant: {tool} hidden: {offered:?}"
            );
        }
        let call = result_of(&bodies, "call-0");
        assert_no_host_route(mode, &offered, &call, &root.checkout_root());
        assert_eq!(
            front.seen(),
            0,
            "{mode:?} keeps no host route, so nothing needing the host is put to the front"
        );
    }
}

/// A grant recorded mid-session — a "yes for this run" at the front — is
/// honoured on the next turn's decision: the second write of the run is not
/// asked again, still lands, and the store the *advertisement* reads (the
/// launch's seeds) is untouched, because the mode surface reads the launch and
/// the deciders read the operator's store.
#[tokio::test]
async fn a_grant_recorded_mid_session_is_honoured_on_the_next_turn() {
    let provider = Scripted::start_with_script(vec![
        Reply::Call {
            name: "write",
            arguments: json!({"path": "first.txt", "content": "first\n"}),
        },
        Reply::Text("wrote the first"),
        Reply::Call {
            name: "write",
            arguments: json!({"path": "second.txt", "content": "second\n"}),
        },
        Reply::Text("wrote the second"),
    ]);
    let fixture = compose_fixture::in_place("twin-grants-recompute", &provider.endpoint).await;
    let front = Fixed::new(Answer::Session);
    let (composition, root) = compose(
        fixture,
        Arc::clone(&front) as Arc<dyn ChatFront>,
        "twin-grants-recompute",
        ApprovalPolicy::Ask,
        &[],
    )
    .await;

    let _ = turn(&composition, "write the first").await;
    assert!(
        offers(&advertised(&provider.bodies()[0]), "write"),
        "an asking run with a front to ask offers the write family"
    );
    assert_eq!(
        front.seen(),
        1,
        "the first write was put to the front, which answered for the run"
    );
    assert!(
        root.checkout_root().join("first.txt").exists(),
        "the answered write landed"
    );

    let _ = turn(&composition, "write the second").await;
    assert_eq!(
        front.seen(),
        1,
        "the grant recorded mid-session pre-answers the next turn's call: nobody is asked again"
    );
    assert!(
        root.checkout_root().join("second.txt").exists(),
        "the pre-answered write still landed"
    );
    assert!(
        !composition
            .job()
            .launch_grants()
            .is_granted(WORKSPACE_WRITE),
        "the launch's seed store is what the mode surface reads, and a session answer does not write it"
    );
}

/// The old download door has no successor in the live catalogue: `http_download`
/// is not advertised under a held fetch grant, and a call to it is refused as
/// unknown — the model learns the name, nothing is fetched, and no destination
/// is written.
#[tokio::test]
async fn the_download_door_is_not_advertised_and_its_call_is_unknown() {
    let provider = Scripted::start_with_script(vec![
        Reply::Call {
            name: "http_download",
            arguments: json!({
                "url": "https://b.example/denied.bin",
                "destination": "denied.bin"
            }),
        },
        Reply::Text("done"),
    ]);
    let fixture =
        compose_fixture::in_place_with_web_search("twin-grants-download", &provider.endpoint).await;
    let front = Fixed::headless();
    let (composition, root) = compose(
        fixture,
        Arc::clone(&front) as Arc<dyn ChatFront>,
        "twin-grants-download",
        ApprovalPolicy::Ask,
        &["fetch:https+a.example"],
    )
    .await;

    let _ = turn(&composition, "download the file").await;

    let bodies = provider.bodies();
    let offered = advertised(&bodies[0]);
    for name in ["http_download", "http_fetch"] {
        assert!(
            !offers(&offered, name),
            "the live catalogue carries no {name}: {offered:?}"
        );
    }
    let refused = result_of(&bodies, "call-0");
    assert!(
        refused_as_unknown(&refused, "http_download"),
        "the foreign-host call was refused as unknown, not served: {refused}"
    );
    assert!(
        !root.checkout_root().join("denied.bin").exists(),
        "the refused call never wrote its destination"
    );
    assert_eq!(
        front.seen(),
        0,
        "nobody was asked about a name that is not offered"
    );
}
