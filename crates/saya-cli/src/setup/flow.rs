//! The `saya setup` interactive adapter (S16): a guided terminal flow over
//! the pure engine — recovery offer (flow_recover), provider and database
//! questions (prompt_provider/prompt_database), plan, review, probes
//! (flow_probe over probe/probe_database/probe_provider), confirm, commit
//! with a real reload check (flow_commit).
//!
//! Non-interactive (no TTY or `--non-interactive`) it never reads stdin: it
//! prints guidance through the error event and exits 2. Nothing is written
//! until the user confirms, and no prompt ever asks for a secret value.

use std::io::{BufRead, IsTerminal, Write};

use crate::cli::Cli;
use crate::render::RenderFormat;

use super::flow_commit;
use super::flow_options::FlowOptions;
use super::flow_probe::{ProbeOutcome, run_probes};
use super::flow_recover::{RecoveryOutcome, check_interrupted};
use super::prompt::{Cancel, Prompter};
use super::prompt_database::collect_database;
use super::prompt_provider::collect_provider;
use super::review;
use super::{SetupDraft, plan};

/// The refusal a script or a pipe gets: guidance, not a hang.
pub(crate) const NON_INTERACTIVE_GUIDANCE: &str = "saya setup is interactive. \
    For a scripted start use `saya config init` (templates) or `saya demo` (sample database).";

/// Every cancellation that did not restore files lands on this line.
const NO_CHANGES: &str = "No files were changed.";

/// The binary entry point: refuses without a terminal, then runs the flow.
pub(crate) fn run(cli: &Cli) -> Result<i32, Box<dyn std::error::Error>> {
    if cli.options.non_interactive || !std::io::stdin().is_terminal() {
        return crate::commands::failure_message(
            2,
            NON_INTERACTIVE_GUIDANCE.to_owned(),
            cli.options.format.into(),
        );
    }
    let mut options = FlowOptions::real(cli.options.clone());
    let mut input = std::io::stdin().lock();
    let mut output = std::io::stdout().lock();
    run_with(&mut options, &mut input, &mut output)
}

/// The guided flow over injected I/O, probes, and reload.
pub(crate) fn run_with(
    options: &mut FlowOptions,
    input: &mut dyn BufRead,
    output: &mut dyn Write,
) -> Result<i32, Box<dyn std::error::Error>> {
    let format: RenderFormat = options.options.format.into();
    let mut prompter = Prompter::new(input, output);
    prompter.say(
        "saya setup — guided configuration.\n\
         Secrets are stored as environment-variable names, never values; \
         nothing is written until you confirm.",
    )?;

    let mut restored = false;
    match check_interrupted(options, &mut prompter)? {
        RecoveryOutcome::Done {
            restored: was_restored,
        } => restored = was_restored,
        RecoveryOutcome::Cancelled => return cancelled(&mut prompter, restored),
        RecoveryOutcome::NoPending => {}
    }

    let provider = match collect_provider(&mut prompter) {
        Ok(draft) => draft,
        Err(Cancel) => return cancelled(&mut prompter, restored),
    };
    let profile = match collect_database(&mut prompter) {
        Ok(draft) => draft,
        Err(Cancel) => return cancelled(&mut prompter, restored),
    };
    if provider.is_none() && profile.is_none() {
        prompter.say("Nothing to configure.")?;
        return cancelled(&mut prompter, restored);
    }
    let draft = SetupDraft { provider, profile };
    let setup_plan = match plan(&options.user_dir, &draft) {
        Ok(planned) => planned,
        Err(error) => return crate::commands::failure_message(2, error.to_string(), format),
    };
    prompter.say(&review::render(&options.user_dir, &setup_plan))?;

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let failures = match run_probes(options, &draft, &mut prompter, &runtime)? {
        ProbeOutcome::Done(failures) => failures,
        ProbeOutcome::Cancelled => return cancelled(&mut prompter, restored),
    };
    if failures > 0 {
        prompter.say(
            "A probe failed (above); the default answer stays no. Writing is still your choice.",
        )?;
    }
    if !matches!(
        prompter.confirm("Write these files? [y/N] ", false),
        Ok(true)
    ) {
        return cancelled(&mut prompter, restored);
    }
    flow_commit::write_files(options, &mut prompter, &draft, &setup_plan, format)
}

/// A cancelled flow exits 0; the line reflects what actually changed.
fn cancelled(
    prompter: &mut Prompter<'_>,
    restored: bool,
) -> Result<i32, Box<dyn std::error::Error>> {
    let message = if restored {
        "The previous files were restored; no other changes were made."
    } else {
        NO_CHANGES
    };
    prompter.say(message)?;
    Ok(0)
}
