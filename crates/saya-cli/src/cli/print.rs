//! The `-p/--print` one-shot surface: one task, one consent, one result.
//!
//! The flags live here rather than in [`GlobalOptions`](super::GlobalOptions):
//! they shape a launch, not the process-wide options every subcommand reads.

use super::{Command, GlobalOptions};
use clap::Args;

/// The bound on a task read from stdin (`-p -`).
pub(crate) const MAX_TASK_BYTES: usize = 16 * 1024;

/// One headless task run in the current directory, then exit.
#[derive(Debug, Clone, Default, Args)]
pub struct PrintOptions {
    /// Run one task headless in the current directory, then exit.
    /// `-` reads the task from stdin, bounded to 16 KiB.
    #[arg(short = 'p', long = "print", value_name = "TASK")]
    pub task: Option<String>,
    /// Use this durable job id instead of a generated `p<millis>-<pid>`.
    #[arg(long = "job-id", value_name = "ID", requires = "task")]
    pub job_id: Option<String>,
    /// Path or subtree the job may edit (repeatable). Needs an edit-capable
    /// mode; with none stated, `-p` may edit the whole workspace (`.`).
    #[arg(long = "write", value_name = "PATH", requires = "task")]
    pub write_paths: Vec<String>,
    /// Stop the job after this many logical model requests. This is a
    /// backstop: the job ends failed (exit 5), not paused.
    #[arg(long = "max-turns", value_name = "N", requires = "task")]
    pub max_turns: Option<usize>,
    /// Stop the job once provider-reported cost reaches this many US dollars.
    #[arg(long = "max-budget-usd", value_name = "USD", requires = "task")]
    pub max_budget_usd: Option<f64>,
}

/// `-p` runs one task and takes no subcommand: a stated subcommand is a
/// second intent, refused rather than resolved by precedence.
pub(crate) fn refuse_subcommand(
    command: Option<&Command>,
    task: Option<&str>,
) -> Result<(), String> {
    if task.is_some() && command.is_some() {
        return Err(
            "`-p/--print` runs one task headless and takes no subcommand: \
                    drop the subcommand or drop `-p`"
                .to_owned(),
        );
    }
    Ok(())
}

/// The session-only flags have no session to bind on `-p`. `--allow-command`
/// and `--deny-command` are the command-rule spelling and stay accepted.
pub(crate) fn refuse_session_flags(options: &GlobalOptions) -> Result<(), String> {
    if options.continue_session {
        return Err(
            "`--continue` continues the interactive REPL session, not a one-shot \
                    task; resume a job by id: `saya job resume <id>`"
                .to_owned(),
        );
    }
    if options.resume.is_some() {
        return Err(
            "`--resume` resumes an interactive session, not a one-shot task; \
                    resume a job by id: `saya job resume <id>`"
                .to_owned(),
        );
    }
    if options.workspace.is_some() {
        return Err(
            "`--workspace` binds the interactive session's workspace, not a one-shot \
                    task: `-p` always runs in the current directory"
                .to_owned(),
        );
    }
    if options.turn_file.is_some() {
        return Err(
            "`--turn-file` reads one interactive session turn, not a one-shot task: \
                    state the task with `-p <TASK>`"
                .to_owned(),
        );
    }
    if !options.allow.is_empty() {
        return Err(
            "`--allow` states the interactive session's launch grants, not a one-shot \
                    task: state `--allow-command <prefix>` to allow a command prefix"
                .to_owned(),
        );
    }
    if !options.deny.is_empty() {
        return Err(
            "`--deny` states the interactive session's deny list, not a one-shot task: \
                    state `--deny-command <prefix>` to refuse a command prefix"
                .to_owned(),
        );
    }
    Ok(())
}

/// `--max-turns` and `--coding-max-logical-model-requests` state the same
/// ceiling; a stated budget must be a finite, non-negative amount.
pub(crate) fn refuse_conflicting_limits(
    print: &PrintOptions,
    options: &GlobalOptions,
) -> Result<(), String> {
    if print.max_turns.is_some() && options.coding_max_logical_model_requests.is_some() {
        return Err(
            "`--max-turns` and `--coding-max-logical-model-requests` state the same \
                    limit: state one of them"
                .to_owned(),
        );
    }
    if let Some(budget) = print.max_budget_usd
        && (!budget.is_finite() || budget < 0.0)
    {
        return Err(format!(
            "`--max-budget-usd` must be a finite, non-negative number, not {budget}"
        ));
    }
    Ok(())
}

/// The task text: the stated value, or stdin when it is `-`, bounded and
/// required to be non-empty.
pub(crate) fn task_text(stated: &str) -> Result<String, String> {
    if stated != "-" {
        if stated.trim().is_empty() {
            return Err(
                "`-p/--print` needs a task: state the task text, or `-p -` to read it from stdin"
                    .to_owned(),
            );
        }
        return Ok(stated.to_owned());
    }
    use std::io::Read as _;
    let mut bytes = Vec::new();
    std::io::stdin()
        .take(MAX_TASK_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("could not read the task from stdin: {error}"))?;
    if bytes.len() > MAX_TASK_BYTES {
        return Err(format!(
            "the task on stdin exceeds {MAX_TASK_BYTES} bytes; pass a shorter task"
        ));
    }
    let text =
        String::from_utf8(bytes).map_err(|_| "the task on stdin is not valid UTF-8".to_owned())?;
    if text.trim().is_empty() {
        return Err("`-p -` read an empty task from stdin".to_owned());
    }
    Ok(text)
}
