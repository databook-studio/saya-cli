//! What a job root and a helper advertise with no evidence in the request and
//! no `[tools] allow` list: each surface's always-on list in `perf_guard`, and
//! nothing else.
//!
//! A name outside the list must be gated behind evidence, or added to the list
//! in review. The two surfaces share one composition and differ only in what
//! the job's own consent narrowed away, so a regression on one is a regression
//! on both — which is why both are measured here, side by side.
#![cfg(any(target_os = "macos", target_os = "linux"))]

use std::sync::Arc;

use saya_agent::{ApprovalPolicy, ToolDefinition};
use saya_config::{ResolvedTools, WebSearchBackend};

use super::direct::RootTools;
use super::direct_tests::{compose_root_data, working_checkout_root_with_request};
use crate::agent::tools::advertisement::Evidence;
use crate::agent::tools::child::{RootWorkspace, tests_root_compose_fixture as fixture};
use crate::agent::tools::shell_policy::{self, BackgroundOwner};
use crate::agent::tools::web::fixture as web_fixture;
use crate::agent::tools::workspace::prepare_coding_workspace_tools;
use crate::perf_guard;

/// The `(name, description)` pairs of a definition list.
fn pairs(definitions: &[ToolDefinition]) -> Vec<(String, String)> {
    definitions
        .iter()
        .map(|definition| (definition.name.clone(), definition.description.clone()))
        .collect()
}

/// The names of a definition list, in composition order.
fn names(definitions: &[ToolDefinition]) -> Vec<&str> {
    definitions
        .iter()
        .map(|definition| definition.name.as_str())
        .collect()
}

/// The job root's advertisement for a task that states nothing: composed with
/// no database surface, no shell, no web lane and no plan, so the only names on
/// it are the ones the composition always carries.
async fn quiet_job(fixture: &fixture::Fixture) -> RootTools {
    working_checkout_root_with_request(fixture, None, None, String::new(), ResolvedTools::default())
        .await
}

/// The helper's advertisement for a task that states nothing: the composition's
/// own list, narrowed by the rule a child applies (`ChildTools::apply_evidence`)
/// and by the child's own admissions, so `list` and the notebook edit are read
/// exactly as the helper's loop reads them.
fn helper_advertisement(workspace: RootWorkspace, notebook: bool) -> Vec<ToolDefinition> {
    let RootWorkspace {
        mut tools,
        definitions,
        ..
    } = workspace;
    let mut definitions = definitions;
    prepare_coding_workspace_tools(&mut definitions);
    tools.apply_evidence(Evidence::new(String::new(), false));
    tools.hold_definitions(&definitions);
    tools.offer_notebook_edit(notebook);
    tools.advertisement()
}

/// The web lane's two definitions, for the description cap: the lane is
/// attached only where a request or the configuration asks for it, but its text
/// is paid on every request once it is.
fn web_definitions() -> Vec<ToolDefinition> {
    let backend = WebSearchBackend::Searxng {
        endpoint: "https://search.example.org/search".to_owned(),
    };
    let net = Arc::new(web_fixture::CannedNet::serving("a page"));
    web_fixture::web_tools(net, Some(backend), ApprovalPolicy::Bypass, None)
        .expect("bypass advertises the web lane")
        .definitions()
}

/// Test 1 — a job whose task states nothing is offered the job's always-on
/// list and nothing else.
#[tokio::test]
async fn a_quiet_jobs_advertisement_is_the_always_on_list() {
    let case = fixture::Fixture::new("guard-job-default", &["tracked.txt"]).await;
    let root = quiet_job(&case).await;
    let definitions = root.definitions();
    let advertised = names(&definitions);
    println!("job default advertisement: {advertised:?}");
    perf_guard::assert_only_listed("job", perf_guard::ALWAYS_ON_JOB, &advertised);
}

/// Test 2 — a helper's advertisement is the helper's always-on list and
/// nothing else: the job's composition minus the tools only a root reaches.
#[tokio::test]
async fn a_quiet_helpers_advertisement_is_the_always_on_list() {
    let case = fixture::Fixture::new("guard-helper-default", &["tracked.txt"]).await;
    let workspace = compose_root_data(&case, ResolvedTools::default(), None).await;
    let advertised = helper_advertisement(workspace, false);
    let advertised = names(&advertised);
    println!("helper default advertisement: {advertised:?}");
    perf_guard::assert_only_listed("helper", perf_guard::ALWAYS_ON_HELPER, &advertised);
}

/// Test 3 — every tool a job and a helper can advertise carries a description
/// within the cap. The two compositions are measured without evidence, and the
/// shell family and the web lane are measured beside them: each of those texts
/// rides a request the moment its lane is attached.
#[tokio::test]
async fn the_coding_surfaces_descriptions_fit_the_cap() {
    let job_case = fixture::Fixture::new("guard-job-cap", &["tracked.txt"]).await;
    let job = quiet_job(&job_case).await;
    let mut definitions = job.definitions();
    definitions.push(shell_policy::definition());
    definitions.extend(shell_policy::background_definitions(BackgroundOwner::Job));
    definitions.extend(web_definitions());

    let helper_case = fixture::Fixture::new("guard-helper-cap", &["tracked.txt"]).await;
    let workspace = compose_root_data(&helper_case, ResolvedTools::default(), None).await;
    definitions.extend(helper_advertisement(workspace, false));

    let measured = pairs(&definitions);
    let longest = measured
        .iter()
        .map(|(name, description)| (description.len(), name.clone()))
        .max();
    println!("coding surfaces' longest description: {longest:?}");
    perf_guard::assert_descriptions_within("job and helper", &measured);
}
