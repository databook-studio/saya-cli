//! Flow step (a): the interrupted-commit check. If `recover::pending` finds a
//! marker, the user is offered restore / finish / quit; the answer is applied
//! immediately or the flow cancels. A restore that does not complete prints
//! every failure plus the marker-kept guidance — never the success text — and
//! fails, so the flow cannot continue configuring a half-restored state. A
//! marker read error is a single warning, never a crash, never an automatic
//! restore.

use super::SetupError;
use super::flow_options::FlowOptions;
use super::prompt::{Cancel, Prompter};
use super::recover;
use super::restore::{RestoreOutcome, RestoreReport, restore};

/// One answer at the recovery prompt.
enum RecoveryChoice {
    Restore,
    Finish,
    Quit,
}

/// The recovery offer: restore the originals, keep the current files, or quit.
fn offer_recovery(prompter: &mut Prompter<'_>) -> Result<RecoveryChoice, Cancel> {
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

/// What step (a) decided: whether files were restored (the flow's later
/// cancel lines must say so), a cancellation, or nothing pending.
pub(crate) enum RecoveryOutcome {
    Done { restored: bool },
    Cancelled,
    NoPending,
}

/// The guidance when the restore did not complete: the pending state stays.
const RESTORE_KEPT_MARKER: &str =
    "The recovery marker was kept; fix the file and run `saya setup` again.";

/// Runs the interrupted-commit check and applies the chosen recovery.
pub(crate) fn check_interrupted(
    options: &FlowOptions,
    prompter: &mut Prompter<'_>,
) -> Result<RecoveryOutcome, Box<dyn std::error::Error>> {
    Ok(match recover::pending(&options.user_dir) {
        Ok(Some(pending)) => match offer_recovery(prompter) {
            Ok(RecoveryChoice::Restore) => match restore(&options.user_dir, &pending) {
                Ok(report) => {
                    for line in restore_lines(&report) {
                        prompter.say(&line)?;
                    }
                    prompter.say("Restored the previous files.")?;
                    RecoveryOutcome::Done { restored: true }
                }
                Err(error) => {
                    say_restore_failure(prompter, &error)?;
                    return Err(error.into());
                }
            },
            Ok(RecoveryChoice::Finish) => {
                recover::finish(&options.user_dir, &pending)?;
                prompter.say("Kept the current files.")?;
                RecoveryOutcome::Done { restored: false }
            }
            _ => RecoveryOutcome::Cancelled,
        },
        Ok(None) => RecoveryOutcome::NoPending,
        Err(error) => {
            eprintln!("warning: could not check for an interrupted `saya setup`: {error}");
            RecoveryOutcome::NoPending
        }
    })
}

/// One line per file: what the restore did to it.
fn restore_lines(report: &RestoreReport) -> Vec<String> {
    report
        .outcomes
        .iter()
        .map(|outcome| match outcome {
            RestoreOutcome::Restored { file } => format!("restored: {file}"),
            RestoreOutcome::Removed { file } => format!("removed: {file}"),
            RestoreOutcome::LeftUnchanged { file, reason } => {
                format!("left unchanged: {file} ({reason})")
            }
        })
        .collect()
}

/// Says every incomplete step and the marker-kept guidance; other restore
/// errors (a failed cleanup) say themselves.
fn say_restore_failure(prompter: &mut Prompter<'_>, error: &SetupError) -> std::io::Result<()> {
    match error {
        SetupError::RestoreIncomplete { failures } => {
            for failure in failures {
                prompter.say(failure)?;
            }
            prompter.say(RESTORE_KEPT_MARKER)?;
        }
        other => prompter.say(&other.to_string())?,
    }
    Ok(())
}
