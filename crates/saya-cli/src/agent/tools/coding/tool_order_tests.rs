//! The coding root's tool order. The tool list a provider caches is a prefix,
//! so a tool added after the order was fixed is appended at the end rather
//! than inserted: every tool that existed before it keeps the position it
//! held, and the prefix is unchanged.
//!
//! `notebook_edit` is that tool — and it is offered only where the workspace
//! holds a notebook, so the list a workspace without one advertises is the
//! list this surface carried before the tool existed, name for name and in
//! the same order.
#![cfg(any(target_os = "macos", target_os = "linux"))]

use std::fs;

use super::direct::RootTools;
use super::direct_tests::working_checkout_root;
use crate::agent::tools::child::tests_root_compose_fixture as fixture;

/// The list this surface advertises before `notebook_edit`: the lean default's
/// job-local checklist and workspace read and write family, in composition
/// order. `list` and `helpers` sit outside the lean default.
const PRE_P11: &[&str] = &["checklist", "read", "glob", "grep", "edit", "write"];

fn names(root: &RootTools) -> Vec<String> {
    root.definitions()
        .into_iter()
        .map(|definition| definition.name)
        .collect()
}

/// With no notebook in the workspace the advertised list is exactly the list
/// without the notebook tool — same names, same order — and the edit is
/// nowhere in it.
#[tokio::test]
async fn a_workspace_without_a_notebook_advertises_the_pre_p11_list() {
    let fixture = fixture::Fixture::new("root-tool-order-absent", &["tracked.txt"]).await;
    let root = working_checkout_root(&fixture).await;
    assert_eq!(
        names(&root),
        PRE_P11,
        "a workspace with no notebook advertises exactly the list it carried before the \
         notebook edit existed"
    );
}

/// A notebook in the workspace puts the edit in the list, last: every tool
/// that existed before it keeps the position it held.
#[tokio::test]
async fn a_notebook_in_the_workspace_appends_the_edit_last() {
    let fixture = fixture::Fixture::new("root-tool-order-present", &["tracked.txt"]).await;
    fs::write(
        fixture.checkout_path.join("analysis.ipynb"),
        b"{\"cells\":[]}",
    )
    .unwrap();
    let root = working_checkout_root(&fixture).await;
    assert_eq!(
        names(&root),
        [PRE_P11, &["notebook_edit"]].concat(),
        "notebook_edit goes last, and nothing before it moved"
    );
}
