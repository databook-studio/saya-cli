//! Flow steps (g)/(h): the write confirmation has already passed; this module
//! builds the reload check, commits, and reports what was written with the
//! next step.

use crate::config::runtime::{RuntimeConfig, load_with_sources};
use crate::render::RenderFormat;

use super::commit;
use super::draft::SetupDraft;
use super::flow_options::{FlowOptions, Reload};
use super::plan::SetupPlan;
use super::prompt::Prompter;

/// Commits `setup_plan` with the real (or injected) reload check and prints
/// the report: what was written, `configuration valid.` when the reload
/// passed, and the next step (`saya` or `saya --profile <name>`).
pub(crate) fn write_files(
    options: &mut FlowOptions,
    prompter: &mut Prompter<'_>,
    draft: &SetupDraft,
    setup_plan: &SetupPlan,
    format: RenderFormat,
) -> Result<i32, Box<dyn std::error::Error>> {
    let reload = build_reload(options, draft);
    let report = match commit(&options.user_dir, setup_plan, reload) {
        Ok(report) => report,
        Err(error) => return crate::commands::failure_message(2, error.to_string(), format),
    };
    if report.written.is_empty() {
        prompter.say("No files needed writing; see the notes above.")?;
    } else {
        for file in &report.written {
            prompter.say(&format!("Wrote {}", options.user_dir.join(file).display()))?;
        }
        prompter.say("configuration valid.")?;
    }
    let next = match &draft.profile {
        Some(profile) => format!("saya --profile {}", profile.name),
        None => "saya".to_owned(),
    };
    prompter.say(&format!("Next: run `{next}` and ask a question."))?;
    Ok(0)
}

/// The reload check: the real one loads the written configuration the way a
/// following command would, over the flow's directories and environment.
fn build_reload(options: &mut FlowOptions, draft: &SetupDraft) -> Reload {
    options.reload.take().unwrap_or_else(|| {
        let env = options.env.clone();
        let user_dir = options.user_dir.clone();
        let cwd = options.cwd.clone();
        let mut cli_options = options.options.clone();
        // The reload verifies the next step the flow prints: when a profile
        // was drafted, `saya --profile <name>` must resolve — an append that
        // leaves two profiles without a default still loads under the new
        // profile.
        cli_options.profile = draft.profile.as_ref().map(|profile| profile.name.clone());
        Box::new(move || {
            load_with_sources(&cli_options, &cwd, &user_dir, env.clone())
                .map(|_: RuntimeConfig| ())
                .map_err(|error| error.to_string())
        })
    })
}
