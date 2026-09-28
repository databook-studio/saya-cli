//! The `saya setup` interactive adapter (S16): a guided terminal flow over
//! the pure engine — recovery offer, provider and database questions, plan,
//! review, bounded probes, confirm, commit with a real reload check.
//!
//! Non-interactive (no TTY or `--non-interactive`) it never reads stdin: it
//! prints guidance through the error event and exits 2. Nothing is written
//! until the user confirms, and no prompt ever asks for a secret value.

use std::collections::BTreeMap;
use std::io::{BufRead, IsTerminal, Write};
use std::path::PathBuf;

use crate::cli::{Cli, GlobalOptions};
use crate::config::runtime::{RuntimeConfig, load_with_sources};
use crate::render::RenderFormat;

use super::draft::ProviderDraft;
use super::probe::FlowProbes;
use super::prompt::{Cancel, Prompter, Step, collect_database, collect_provider};
use super::{SetupDraft, commit, plan, recover, review};

/// The refusal a script or a pipe gets: guidance, not a hang.
pub(crate) const NON_INTERACTIVE_GUIDANCE: &str = "saya setup is interactive. \
    For a scripted start use `saya config init` (templates) or `saya demo` (sample database).";

/// Every cancellation that did not restore files lands on this line.
const NO_CHANGES: &str = "No files were changed.";

/// The reload check `commit` runs after publishing the files.
pub(crate) type Reload = Box<dyn FnMut() -> Result<(), String> + 'static>;

/// Everything the flow needs, injectable so tests drive it without a
/// terminal: the CLI options, the directories, the environment, the reload
/// closure (`None` builds the real `load_with_sources` check), and the
/// probes.
pub(crate) struct FlowOptions {
    pub(crate) options: GlobalOptions,
    pub(crate) user_dir: std::path::PathBuf,
    pub(crate) cwd: std::path::PathBuf,
    pub(crate) env: BTreeMap<String, String>,
    pub(crate) reload: Option<Reload>,
    pub(crate) probes: FlowProbes,
}

impl FlowOptions {
    /// The real options: user config dir, process env, real probes, real reload.
    pub(crate) fn real(options: GlobalOptions) -> Self {
        let env = crate::config::sources::process_env();
        Self {
            probes: FlowProbes::real(&env),
            env,
            user_dir: crate::config::sources::user_config_dir(),
            cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            options,
            reload: None,
        }
    }
}

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
    match recover::pending(&options.user_dir) {
        Ok(Some(pending)) => match offer_recovery(&mut prompter) {
            Ok(RecoveryChoice::Restore) => {
                recover::restore(&options.user_dir, &pending)?;
                restored = true;
                prompter.say("Restored the previous files.")?;
            }
            Ok(RecoveryChoice::Finish) => {
                recover::finish(&options.user_dir, &pending)?;
                prompter.say("Kept the current files.")?;
            }
            _ => return cancelled(&mut prompter, restored),
        },
        Ok(None) => {}
        Err(error) => {
            eprintln!("warning: could not check for an interrupted `saya setup`: {error}")
        }
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
    let mut failures = 0usize;
    if let Some(profile) = &draft.profile {
        prompter.say("Probing the database (up to 15 seconds)...")?;
        let result = runtime.block_on((options.probes.database)(&profile.profile));
        prompter.say(&result.message)?;
        failures += usize::from(!result.ok);
    }
    if let Some(provider_draft) = &draft.provider {
        let consented = match prompter.confirm(&provider_consent(provider_draft), false) {
            Ok(true) => true,
            Ok(false) => {
                prompter.say("Provider probe skipped.")?;
                false
            }
            Err(Cancel) => return cancelled(&mut prompter, restored),
        };
        if consented {
            prompter.say("Probing the provider (up to 15 seconds)...")?;
            let result = runtime.block_on((options.probes.provider)(provider_draft));
            prompter.say(&result.message)?;
            failures += usize::from(!result.ok);
        }
    }

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
    let reload = options.reload.take().unwrap_or_else(|| {
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
    });
    let report = match commit(&options.user_dir, &setup_plan, reload) {
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

/// One answer at the recovery prompt (flow step (a)).
enum RecoveryChoice {
    Restore,
    Finish,
    Quit,
}

/// The recovery offer for an interrupted commit: restore the originals, keep
/// the current files, or quit.
fn offer_recovery(prompter: &mut Prompter<'_>) -> Step<RecoveryChoice> {
    prompter.ask(
        "An interrupted `saya setup` was found. [r]estore the previous files / \
         [f]inish (keep the current files) / [q]uit: ",
        |line| match line.trim().to_ascii_lowercase().as_str() {
            "r" | "restore" => Ok(RecoveryChoice::Restore),
            "f" | "finish" => Ok(RecoveryChoice::Finish),
            "q" | "quit" => Ok(RecoveryChoice::Quit),
            other => Err(format!("unrecognized answer {other:?}; answer r, f, or q")),
        },
    )
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
