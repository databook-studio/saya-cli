//! The drift check on the live catalogue: every name the job root advertises is
//! one a `[tools] allow` list can name — or drop.
//!
//! Twin of `agent::tools::tools_allow_drift_tests`, whose collection includes
//! the old session's own members (`interactive/session_definitions::*`). The
//! names here come from the constructors the job root itself composes, so a
//! definition added to the live catalogue without a catalog entry still moves
//! the test.

use saya_agent::ApprovalPolicy;
use saya_config::{ResolvedTools, is_database_tool, is_governed_tool};

use super::*;
use crate::agent::tools::shell_policy::BackgroundOwner;

/// Every name the job root's catalogue carries, from the constructors it
/// composes them with: the shell family, the web lane, and the file tools.
fn root_catalogue() -> Vec<String> {
    let mut names = vec![core_shell().name, core_web_fetch().name];
    names.extend(
        crate::agent::tools::shell_policy::background_definitions(BackgroundOwner::Job)
            .into_iter()
            .map(|definition| definition.name),
    );
    for name in ["write", "edit"] {
        names.push(core_file_tool(name).name);
    }
    names
}

/// Each core name is governed — a list can name it — and never connection-tied:
/// these are capabilities the surface composes for itself, so an allow list must
/// be able to take any of them away.
#[test]
fn every_core_catalogue_name_is_governed_by_the_allow_list() {
    let names = root_catalogue();
    assert!(
        names.len() >= 6,
        "the collection must reach the real catalogue, not a stub: {names:?}"
    );
    for name in &names {
        assert!(
            is_governed_tool(name),
            "{name:?} is advertised by the job root but is in neither the governed nor the \
             connection-tied catalog in `saya-config::tools::catalog`: a `[tools] allow` list \
             could neither name it nor explain why it cannot. Classify it in the catalog."
        );
        assert!(
            !is_database_tool(name),
            "{name:?} is a capability the job root composes itself, not a connection-tied tool"
        );
    }
}

/// A stated allow list that omits a core name drops it, and a list that names it
/// keeps it: the name is one the operator can actually take away.
#[test]
fn a_stated_allow_list_can_drop_every_core_name() {
    let names = root_catalogue();
    for name in &names {
        let keeping = ResolvedTools::from_stated(Some(std::slice::from_ref(name)))
            .expect("a valid list")
            .admits(name);
        assert!(keeping, "naming {name:?} must keep it");
        let dropping = ResolvedTools::from_stated(Some(&["read".to_owned()]))
            .expect("a valid list")
            .admits(name);
        assert!(
            !dropping,
            "{name:?} survives a list that does not name it: the list gates nothing"
        );
    }
}

/// The shell is the one tool whose own lane decision survives the allow list:
/// `never` composes none of it, which is the mode's removal and not the list's.
#[test]
fn the_shell_is_composed_only_where_a_mode_serves_it() {
    for mode in [
        ApprovalPolicy::Ask,
        ApprovalPolicy::AcceptEdits,
        ApprovalPolicy::Bypass,
        ApprovalPolicy::ReadOnly,
    ] {
        assert!(
            crate::agent::tools::shell_policy::advertised(mode),
            "{mode:?} serves a shell, so the catalogue carries the name"
        );
    }
    assert!(
        !crate::agent::tools::shell_policy::advertised(ApprovalPolicy::Never),
        "`never` advertises no shell"
    );
}
