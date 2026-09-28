//! Flow step (a): the interrupted-commit check. If `recover::pending` finds a
//! marker, the user is offered restore / finish / quit; the answer is applied
//! immediately or the flow cancels. A marker read error is a single warning,
//! never a crash, and never an automatic restore.

use super::flow_options::FlowOptions;
use super::prompt::{Cancel, Prompter};
use super::recover;

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

/// Runs the interrupted-commit check and applies the chosen recovery.
pub(crate) fn check_interrupted(
    options: &FlowOptions,
    prompter: &mut Prompter<'_>,
) -> Result<RecoveryOutcome, Box<dyn std::error::Error>> {
    Ok(match recover::pending(&options.user_dir) {
        Ok(Some(pending)) => match offer_recovery(prompter) {
            Ok(RecoveryChoice::Restore) => {
                recover::restore(&options.user_dir, &pending)?;
                prompter.say("Restored the previous files.")?;
                RecoveryOutcome::Done { restored: true }
            }
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
