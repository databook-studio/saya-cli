//! The drift test: every tool name any surface can advertise is either in the
//! governed catalog `[tools] allow` / `--tools` may name, in the
//! connection-tied set the list must not name, or on the one-name exempt list
//! below.
//!
//! The catalog in `saya-config` is the one vocabulary, and the allow list is
//! applied by filtering advertisements. A new tool whose name is in neither
//! list would therefore be removable by no list at all — silently ungoverned
//! by accident rather than by the deliberate decision the database set records.
//! This test is what fails when that happens.
//!
//! The names are collected from every composition a surface builds — the
//! database composition, the interactive session's own members, the coding
//! root, the helper, the `saya run` step builder, and the doctor's probe — not
//! from a second hand-written list, so adding a definition is what moves this
//! test.

use std::sync::OnceLock;

use saya_agent::{ApprovalPolicy, CancellationToken, ToolDefinition};
use saya_config::{is_database_tool, is_governed_tool};

use super::DatabaseTools;

/// The one name a surface advertises that is deliberately outside the catalog,
/// with the reason it is exempt. `saya job doctor` composes a single
/// synthetic definition to prove the provider can complete a tool call: it
/// takes one fixed argument, touches nothing, and is no capability an operator
/// would grant or withhold, so no `[tools] allow` list may name it and none can
/// remove it.
const EXEMPT: &[&str] = &[crate::commands::coding::READINESS_PROBE_TOOL];

fn names(definitions: Vec<ToolDefinition>) -> Vec<String> {
    definitions
        .into_iter()
        .map(|definition| definition.name)
        .collect()
}

/// The widest database composition: every optional member switched on.
fn database_names() -> Vec<String> {
    names(DatabaseTools::definitions(true, true, true, true))
}

/// The interactive session's own members: the write-shaped and network tools
/// it rewords for its surface (`interactive/session_definitions`), plus the
/// job-local checklist.
fn session_names() -> Vec<String> {
    names(vec![
        crate::interactive::session_definitions::http_fetch(),
        crate::interactive::session_definitions::http_download(),
        crate::interactive::session_definitions::run_command(),
        crate::interactive::session::tasks::render::checklist_definition(),
    ])
}

/// The workspace family every checkout-bound surface composes over its own
/// root: the child toolset's reads and writes (`child/executor.rs`), the
/// `delete` the composition root attaches with the coding operations, the
/// helper's `shell`, and its web lane — both tools, because the catalog is the
/// vocabulary of names, not of what a run happened to turn on.
fn helper_names() -> Vec<String> {
    let mut definitions = Vec::new();
    super::workspace::append_definitions(&mut definitions, true);
    super::workspace::append_notebook_edit(&mut definitions, true);
    definitions.push(super::workspace::delete_definition());
    definitions.push(crate::interactive::session::tasks::render::checklist_definition());
    definitions.extend(super::shell_policy::background_definitions(
        super::shell_policy::BackgroundOwner::Helper,
    ));
    definitions.push(super::web::fetch_definition());
    definitions.push(super::web::search::definition());
    names(definitions)
}

/// The coding root: the coordinator's own definitions, the delegation tool,
/// the job-local checklist, the same workspace family over W, the root
/// shell — the whole background family (`shell_policy::background_definitions`,
/// which the root advertises in place of `definition`) — and the root's web
/// lane, both tools.
fn coding_root_names() -> Vec<String> {
    let mut definitions = crate::agent::tools::coding::definitions(None);
    definitions.push(crate::agent::tools::coding::task_tool::definition(
        &saya_config::HelperAgents::default(),
    ));
    definitions.push(crate::interactive::session::tasks::render::checklist_definition());
    let mut workspace = Vec::new();
    super::workspace::append_definitions(&mut workspace, true);
    super::workspace::append_notebook_edit(&mut workspace, true);
    definitions.extend(workspace);
    definitions.push(super::workspace::delete_definition());
    definitions.extend(super::shell_policy::background_definitions(
        super::shell_policy::BackgroundOwner::Job,
    ));
    definitions.push(super::web::fetch_definition());
    definitions.push(super::web::search::definition());
    // The plan gate's own tool, advertised by a plan-first root while the
    // write surface is closed.
    definitions.push(crate::agent::tools::coding::plan_gate::PlanGate::definition());
    // The root's model-facing rewind, advertised for the whole run.
    definitions.push(crate::agent::tools::coding::RootRewind::definition());
    names(definitions)
}

/// The MCP family: the names a configured server's tools are advertised under
/// by the coding root and the interactive session.
///
/// No fixed list can hold them — they come from third-party servers the
/// configuration names — so the catalog governs them by *shape*
/// (`saya_config::is_governed_tool`). This is where a real listing is put
/// through the same assertion as every hand-written name above: a surface that
/// advertised an `mcp__` name the catalog did not govern would make that tool
/// removable by no `[tools] allow` list at all, which is the accident this
/// whole file exists to catch.
///
/// Composed once, against the shell fixture the client's own tests use, so the
/// three collecting tests below share one started server.
fn mcp_names() -> Vec<String> {
    static NAMES: OnceLock<Vec<String>> = OnceLock::new();
    NAMES
        .get_or_init(|| {
            use crate::mcp::client::{McpTools, support::Fixture, support::TIMEOUTS};
            let fixture = Fixture::new("drift");
            let servers = fixture.servers(&[], TIMEOUTS);
            let resolver = fixture.resolver(&[]);
            let lane = McpTools::connect(
                &servers,
                &resolver,
                ApprovalPolicy::Bypass,
                None,
                &crate::agent::tools::consent::Caller::Session,
                CancellationToken::new(),
            )
            .expect("a configured server composes a lane");
            lane.definitions()
                .into_iter()
                .map(|definition| definition.name)
                .collect()
        })
        .clone()
}

/// Every name a surface can put in front of the model.
fn composed_names() -> Vec<String> {
    let mut names = database_names();
    names.extend(session_names());
    names.extend(helper_names());
    names.extend(coding_root_names());
    names.extend(crate::commands::run::tools_tests::widest_composed_names());
    names.extend(mcp_names());
    names.push(crate::commands::coding::READINESS_PROBE_TOOL.to_string());
    names
}

/// Every composed name is governed, connection-tied, or the one exempt name.
#[test]
fn every_composed_tool_name_is_in_the_catalog() {
    let names = composed_names();
    assert!(
        names.len() > 20,
        "the collection must produce the real surfaces, not a stub: {names:?}"
    );
    for name in &names {
        assert!(
            is_governed_tool(name) || is_database_tool(name) || EXEMPT.contains(&name.as_str()),
            "{name:?} is advertised but is in neither the governed nor the connection-tied \
             catalog in `saya-config::tools::catalog`, and is not on this test's exempt list: a \
             `[tools] allow` list could neither name it nor explain why it cannot. Classify it \
             in the catalog."
        );
    }
}

/// The exempt list is the doctor's probe and nothing else: an exempt name may
/// not be governed or connection-tied (the list cannot be used to smuggle a
/// real tool out of the catalog), and the collection must actually reach it.
#[test]
fn the_exempt_list_is_only_the_doctors_probe() {
    assert_eq!(
        EXEMPT,
        &[crate::commands::coding::READINESS_PROBE_TOOL],
        "one name, with the reason recorded above"
    );
    for name in EXEMPT {
        assert!(
            !is_governed_tool(name) && !is_database_tool(name),
            "{name} is a catalogued tool and must not be exempt from the allow list"
        );
        assert!(
            composed_names().contains(&(*name).to_string()),
            "the collection must reach {name}, or this exemption is stale"
        );
    }
}

/// The members a surface composes that the widest database composition does
/// not carry are governed — each one is a capability the operator may take
/// away — and the only names outside the catalog are the connection-tied ones.
#[test]
fn nothing_composed_is_ungoverned_except_the_connection_tied_set() {
    let names = composed_names();
    for name in &names {
        if is_database_tool(name) || EXEMPT.contains(&name.as_str()) {
            continue;
        }
        assert!(
            is_governed_tool(name),
            "{name} is a capability a surface composes for itself, so a list must be able to \
             take it away"
        );
    }
}

/// An MCP name is governed by shape, and the server's own entry — the word an
/// allow list states to keep a whole server — is a name the list accepts. The
/// config tests state this at the vocabulary level; this states it against a
/// real listing, which is where the two could drift apart.
#[test]
fn every_advertised_mcp_name_is_governed_by_its_shape() {
    let names = mcp_names();
    assert_eq!(
        names.len(),
        3,
        "the fixture server advertises three tools: {names:?}"
    );
    for name in &names {
        assert!(is_governed_tool(name), "{name} must be governed");
        assert!(!is_database_tool(name), "{name} is not connection-tied");
        assert!(
            saya_config::is_mcp_tool_call_name(name),
            "{name} must be the mcp__<server>__<tool> form a call carries"
        );
        let server = saya_config::mcp_server_of(name).expect("an MCP name has a server");
        assert!(
            saya_config::is_mcp_allow_name(server),
            "the whole-server word {server:?} must be one a list can state"
        );
        assert!(
            saya_config::ResolvedTools::from_stated(Some(&[server.to_owned()]))
                .expect("a valid list")
                .admits(name),
            "naming {server:?} must keep {name}"
        );
    }
}

/// The connection-tied names are exactly the ones a list refuses, and every
/// one of them is reached by some composition.
#[test]
fn the_connection_tied_names_are_the_database_composition() {
    let names = composed_names();
    for name in saya_config::DATABASE_TOOLS {
        assert!(
            names.contains(&(*name).to_string()),
            "{name} is in the connection-tied catalog but no composition advertises it: the \
             catalog and the surface have drifted apart"
        );
    }
}
