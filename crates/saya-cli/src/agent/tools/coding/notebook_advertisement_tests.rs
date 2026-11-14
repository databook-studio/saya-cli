//! The coding root offers `notebook_edit` only where the workspace actually
//! holds a notebook. It is the one tool decided by a property of the tree
//! rather than of the composition, so the decision is the bounded presence
//! check, re-read at the start of each turn until it answers yes and latched
//! from then on: a notebook created during the run makes the tool appear, and
//! the list never flaps back.
//!
//! The two refusals that follow from it are here too: a call while the list
//! leaves the edit out is the unknown-tool refusal every uncomposed name gets,
//! and the job's `[tools] allow` list still takes the name away where a
//! notebook does exist.
#![cfg(any(target_os = "macos", target_os = "linux"))]

use std::fs;

use saya_agent::{ToolError, ToolExecutor};

use super::direct::RootTools;
use super::direct_tests::working_checkout_root;
use crate::agent::tools::child::tests_root_compose_fixture as fixture;

/// The notebook the run itself would write, in the working checkout W.
const NOTEBOOK: &str = "analysis.ipynb";

fn write_notebook(fixture: &fixture::Fixture) {
    fs::write(fixture.checkout_path.join(NOTEBOOK), b"{\"cells\":[]}").unwrap();
}

fn names(root: &RootTools) -> Vec<String> {
    root.definitions()
        .into_iter()
        .map(|definition| definition.name)
        .collect()
}

/// With no notebook the edit is not advertised, and a call to it is refused
/// as an unknown tool rather than served — the same refusal a name this
/// surface never composed gets.
#[tokio::test]
async fn a_call_to_the_edit_without_a_notebook_is_an_unknown_tool() {
    let fixture = fixture::Fixture::new("notebook-refusal", &["tracked.txt"]).await;
    let root = working_checkout_root(&fixture).await;
    assert!(
        !names(&root).contains(&"notebook_edit".to_string()),
        "no notebook, no notebook edit"
    );
    let refused = root
        .execute(
            "notebook_edit",
            serde_json::json!({"path": NOTEBOOK, "mode": "delete", "index": 0}),
        )
        .await
        .expect_err("an unadvertised name must refuse, not be served");
    assert!(
        matches!(refused, ToolError::UnsupportedTool),
        "the refusal is the unknown-tool one: {refused:?}"
    );
}

/// A notebook created during the run puts the edit in the next turn's list —
/// the loop re-reads the advertisement before every request — and once it has
/// appeared it stays: the list grows once for this cause and is stable after.
#[tokio::test]
async fn a_notebook_created_during_the_run_appears_and_then_stays() {
    let fixture = fixture::Fixture::new("notebook-later", &["tracked.txt"]).await;
    let root = working_checkout_root(&fixture).await;
    assert!(
        !names(&root).contains(&"notebook_edit".to_string()),
        "the run starts with no notebook in the workspace"
    );

    write_notebook(&fixture);
    let grown = names(&root);
    assert_eq!(
        grown.last().map(String::as_str),
        Some("notebook_edit"),
        "the next turn's list ends with the edit: {grown:?}"
    );

    // The latch is monotonic: a notebook removed after the fact does not take
    // the tool back out, so the cached prefix changes once for this cause.
    fs::remove_file(fixture.checkout_path.join(NOTEBOOK)).unwrap();
    assert_eq!(
        names(&root),
        grown,
        "once advertised the edit stays advertised"
    );
}

/// `[tools] allow` still governs the name: a list that removes it takes it
/// away from the advertisement — and so from the door — even where the
/// workspace holds a notebook.
#[tokio::test]
async fn the_tools_allow_list_still_governs_the_notebook_edit() {
    let fixture = fixture::Fixture::new("notebook-allow", &["tracked.txt"]).await;
    write_notebook(&fixture);
    let mut root = working_checkout_root(&fixture).await;
    assert!(
        names(&root).contains(&"notebook_edit".to_string()),
        "the notebook puts the edit in the list before any list narrows it"
    );
    let stated =
        saya_config::ResolvedTools::from_stated(Some(&["read".to_string()])).expect("a valid list");
    let RootTools::WorkingCheckout(direct) = &mut root else {
        panic!("the fixture composes a working checkout");
    };
    crate::agent::tools::narrow(&mut direct.defs, &stated);
    assert!(direct.advertises("read"), "the list keeps `read`");
    assert!(
        !direct.advertises("notebook_edit"),
        "the list removes the notebook edit even where a notebook exists"
    );
}
