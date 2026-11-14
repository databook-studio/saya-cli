//! `saya -p` dispatch: validate the one-shot surface, resolve its consent,
//! and run the job on a current-thread runtime.

use crate::{
    cli::{Cli, print},
    commands::{
        self,
        coding::{OneShotRequest, launch_plan, resolve_mode},
    },
    config,
    render::RenderFormat,
};
use std::path::Path;

/// Runs the stated one-shot task. Guards and consent refusals answer with the
/// usage exit code before anything is opened; a job that starts answers with
/// the exit code its terminal state states.
pub(super) fn dispatch(cli: Cli) -> Result<i32, Box<dyn std::error::Error>> {
    let stated_format: RenderFormat = cli.options.format.into();
    for refusal in [
        print::refuse_session_flags(&cli.options),
        print::refuse_conflicting_limits(&cli.print, &cli.options),
    ] {
        if let Err(message) = refusal {
            return commands::failure_message(2, message, stated_format);
        }
    }
    let task = match print::task_text(cli.print.task.as_deref().unwrap_or_default()) {
        Ok(task) => task,
        Err(message) => return commands::failure_message(2, message, stated_format),
    };

    // Fold the one-shot flags into the one global options value the runtime
    // loader reads: `--max-turns` is the logical-request ceiling and
    // `--max-budget-usd` is the same `[limits.job]` key config states.
    let mut options = cli.options.clone();
    if let Some(turns) = cli.print.max_turns {
        options.coding_max_logical_model_requests = Some(turns);
    }
    options.max_cost_usd = cli.print.max_budget_usd;
    let runtime = config::runtime::load(&options, Path::new("."))?;
    let format = config::runtime::format_name(&options, &runtime.resolved);

    let mode = match resolve_mode(
        options.permission_mode.as_deref(),
        options.approval_mode.as_deref(),
        false,
    ) {
        Ok(mode) => mode,
        Err(message) => return commands::failure_message(2, message, format),
    };
    // P6's seam: the resolved command rules are read at shell time, so they
    // change no field of the plan; the value is stated truthfully anyway.
    let rules = &runtime.resolved.permissions.rules;
    let has_rules = !rules.allow.is_empty() || !rules.deny.is_empty();
    let plan = match launch_plan(mode, &cli.print.write_paths, has_rules) {
        Ok(plan) => plan,
        Err(message) => return commands::failure_message(2, message, format),
    };
    let request = OneShotRequest {
        task,
        job_id: cli.print.job_id.clone(),
        // The plan resolved the effective scope: an edit-capable `-p` with no
        // `--write` gets the whole workspace, and a stated scope narrows it.
        // Every later check reads this request.
        write_paths: plan.write_paths.clone(),
        mode,
    };

    let result = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(crate::commands::coding::oneshot_cli::run(
            request, plan, format, runtime,
        ));
    match result {
        Ok(code) => Ok(code),
        // Nothing ran: an open or start refusal is a usage answer, matching
        // the plan's "nothing started" row.
        Err(error) => commands::failure_message(2, error.to_string(), format),
    }
}
