//! Flow step (f): the probes, one at a time — the database always, the
//! provider only with explicit consent (`[y/N]`). Each probe is bounded to
//! 15 seconds. `Done` carries how many probes failed (a failed probe does
//! not block writing; the confirm note says so and the default stays No).

use super::draft::{ProviderDraft, SetupDraft};
use super::flow_options::FlowOptions;
use super::probe_database::{SSO_CONSENT_PROMPT, needs_sso_consent, probe_window_label};
use super::prompt::{Cancel, Prompter};

/// The probes, one at a time: the database always, the provider only with
/// explicit consent. `Done` carries how many failed (the confirm note says
/// so); `Cancelled` is a consent prompt's EOF.
pub(crate) enum ProbeOutcome {
    Done(usize),
    Cancelled,
}

pub(crate) fn run_probes(
    options: &FlowOptions,
    draft: &SetupDraft,
    prompter: &mut Prompter<'_>,
    runtime: &tokio::runtime::Runtime,
) -> Result<ProbeOutcome, Box<dyn std::error::Error>> {
    let mut failures = 0usize;
    if let Some(profile) = &draft.profile {
        // Browser-SSO opens a browser only on explicit consent; declining
        // skips the probe and says so.
        let skip = if needs_sso_consent(&profile.profile) {
            match prompter.confirm(SSO_CONSENT_PROMPT, false) {
                Ok(true) => false,
                Ok(false) => {
                    prompter.say("Snowflake SSO probe skipped (not probed).")?;
                    true
                }
                Err(Cancel) => return Ok(ProbeOutcome::Cancelled),
            }
        } else {
            false
        };
        if !skip {
            prompter.say(&format!(
                "Probing the database (up to {})...",
                probe_window_label(&profile.profile)
            ))?;
            let result = runtime.block_on((options.probes.database)(&profile.profile));
            prompter.say(&result.message)?;
            failures += usize::from(!result.ok);
        }
    }
    if let Some(provider_draft) = &draft.provider {
        let consented = match prompter.confirm(&provider_consent(provider_draft), false) {
            Ok(true) => true,
            Ok(false) => {
                prompter.say("Provider probe skipped.")?;
                false
            }
            Err(Cancel) => return Ok(ProbeOutcome::Cancelled),
        };
        if consented {
            prompter.say("Probing the provider (up to 15 seconds)...")?;
            let result = runtime.block_on((options.probes.provider)(provider_draft));
            prompter.say(&result.message)?;
            failures += usize::from(!result.ok);
        }
    }
    Ok(ProbeOutcome::Done(failures))
}

/// The provider probe consent line, with the endpoint it would reach.
fn provider_consent(draft: &ProviderDraft) -> String {
    let target = draft
        .base_url
        .as_deref()
        .unwrap_or("the provider's default endpoint");
    format!(
        "Send one test request to {} at {target}? It sends only the word 'ping' — \
         no schema or data. [y/N] ",
        draft.provider.as_str()
    )
}
