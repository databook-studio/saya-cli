use super::super::run_binding::Review;
use super::super::{EXIT_INVESTIGATION_ERROR, parse_investigation_id, store_failure};
use super::super::{bindings, run_report};
use super::support;
use super::{InitialReplay, RunRequest, Stage};
use crate::commands::output::{failure, failure_message};
use crate::config::runtime::RuntimeConfig;
use crate::profile_identity::profile_identity;
use crate::render::RenderFormat;
use saya_store::InvestigationRepository;
use saya_types::ConnectionError;
use std::time::Instant;

pub(super) async fn prepare<'a>(
    repo: &InvestigationRepository,
    runtime: &'a RuntimeConfig,
    format: RenderFormat,
    request: &RunRequest<'_>,
) -> Result<Stage<InitialReplay<'a>>, Box<dyn std::error::Error>> {
    macro_rules! finish {
        ($result:expr) => {
            return match $result {
                Ok(outcome) => Ok(super::Stage::Finished(Box::new(outcome))),
                Err(error) => Err(error),
            }
        };
    }

    if support::is_cancelled(request.cancellation) {
        finish!(support::stopped(format));
    }
    // Flag-usage refusals come first: they are command-line errors, checked
    // before any store, connection, or query work (invariant 1).
    if let Some(message) = run_report::usage_error(request) {
        finish!(support::no_replay(failure_message(
            EXIT_INVESTIGATION_ERROR,
            message,
            format
        )));
    }
    let id = match parse_investigation_id(request.id) {
        Ok(id) => id,
        Err((code, message)) => finish!(support::no_replay(failure_message(code, message, format))),
    };
    let definition = match repo.get(&id) {
        Ok(definition) => definition,
        Err(error) => finish!(support::no_replay(store_failure(
            error,
            id.as_str(),
            format
        ))),
    };
    // The parameter bindings parse against the declared specs (which the
    // definition read above supplies) before any profile or connection
    // work (invariant 2) — no connection is ever made with a bad binding:
    // unknown names, a malformed value, and a missing required parameter
    // are usage refusals.
    let bound = match bindings::bind_values(&definition.parameters, &request.params) {
        Ok(bound) => bound,
        Err((code, message)) => finish!(support::no_replay(failure_message(code, message, format))),
    };
    let binding = match repo.get_binding(&id) {
        Ok(binding) => binding,
        Err(error) => finish!(support::no_replay(store_failure(
            error,
            id.as_str(),
            format
        ))),
    };
    // A machine-driven replay (the MCP tool) can never establish review
    // authority (FIX-S6a): an UNBOUND investigation refuses here — an
    // imported team file leaves the binding to a human, and success would
    // otherwise write one for a model-supplied profile. A present binding —
    // save-time or human-refreshed, fingerprint or not — runs as before,
    // still gated for staleness against the explicit target below, and is
    // never written by this run (id only, no values).
    if request.machine_driven && binding.is_none() {
        finish!(support::no_replay(failure_message(
            EXIT_INVESTIGATION_ERROR,
            format!(
                "this investigation has no local review binding on this machine; review it once with `saya investigation run {} --connection <profile>` (a human step) before a tool can replay it",
                id.as_str(),
            ),
            format,
        )));
    }
    // Target = `--connection` else the binding's profile; never the
    // active or default profile (invariant 1).
    let Some(target) = request
        .connection
        .or(binding.as_ref().map(|b| b.profile.as_str()))
    else {
        finish!(support::no_replay(failure_message(
            EXIT_INVESTIGATION_ERROR,
            "no local connection mapped: pass --connection <profile>".to_string(),
            format,
        )));
    };
    let profile = match runtime.named_profile(target) {
        Ok(profile) => profile,
        Err(error) => {
            finish!(support::no_replay(failure(
                3,
                ConnectionError::invalid_configuration(error.to_string()),
                format,
            )));
        }
    };
    if profile.dialect() != definition.dialect {
        let message = format!(
            "dialect mismatch: the investigation is saved for {} but profile {target:?} is {}",
            definition.dialect.as_str(),
            profile.dialect().as_str(),
        );
        finish!(support::no_replay(failure_message(
            EXIT_INVESTIGATION_ERROR,
            message,
            format
        )));
    }
    let review = Review {
        target: target.to_string(),
        identity: profile_identity(target, profile, &runtime.cache_scope)
            .as_str()
            .to_owned(),
        fingerprint: None,
    };

    let started = Instant::now();
    Ok(Stage::Continue(super::InitialReplay {
        profile,
        id,
        definition,
        bound,
        binding,
        review,
        started,
    }))
}
