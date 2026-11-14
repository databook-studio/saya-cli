//! The `shell` tool's advertised schema: the definitions every surface
//! advertises, the bounds they state, and the name table that routes them.
//!
//! A shell that can own background runs advertises three definitions together
//! ([`background_definitions`]): the `shell` that starts one, and the two
//! calls that reach it afterwards — `shell_output` and `shell_kill`. They are
//! one family because a start with no reader would be a promise the model
//! could not keep, and [`definition`] — the foreground shell every other
//! surface advertises — carries no `run_in_background` at all.
//!
//! What a call may send is [`arguments`](super::arguments)'s, and the bounds
//! stated here are the numbers it enforces.

use saya_agent::{LocalStateEffect, ToolConcurrency, ToolDefinition, ToolEffect};
use serde_json::{Value, json};

/// The command's byte ceiling: the harness shell's own bound, not a copy, so
/// the advertised limit and the enforcing one cannot drift.
pub(crate) const MAX_COMMAND_BYTES: usize = saya_harness::runner::shell::MAX_COMMAND_BYTES;
/// The workdir string's byte ceiling.
pub(crate) const MAX_WORKDIR_BYTES: usize = 512;
/// The timeout ceiling in milliseconds: 600 s. The default a call gets when it
/// states none is 120 s, so a call may widen the default up to this ceiling.
pub(crate) const MAX_TIMEOUT_MILLIS: u64 = 600_000;

/// Which of the shell family's operations a tool name asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Operation {
    /// `shell`: run one command, waited for or detached.
    Run,
    /// `shell_output`: read a background run.
    Output,
    /// `shell_kill`: end a background run.
    Kill,
}

/// The operation `name` asks for, or `None` where the name is not the shell's.
/// The one name table every surface that routes the family shares, so a
/// dispatch and an advertisement cannot disagree about which tool a word
/// names.
pub(crate) fn operation(name: &str) -> Option<Operation> {
    match crate::agent::tools::workspace::canonical_name(name) {
        "shell" => Some(Operation::Run),
        "shell_output" => Some(Operation::Output),
        "shell_kill" => Some(Operation::Kill),
        _ => None,
    }
}

/// The surface whose shell owns a background run. It decides the one sentence
/// that says how long a detached command lives, so the model reads the truth
/// for the surface it is on rather than one surface's rule on all of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BackgroundOwner {
    /// A coding job's root shell: runs end with the job.
    Job,
    /// A helper's shell: runs end with the helper.
    Helper,
    /// The interactive session's shell: runs end with the session.
    Session,
}

impl BackgroundOwner {
    /// How the run ends besides the command exiting or the model killing it.
    fn ending(self) -> &'static str {
        match self {
            Self::Job => "the job ends",
            Self::Helper => "the helper ends",
            Self::Session => "the session ends",
        }
    }
}

/// The `shell` definition every surface advertises, byte-identical wherever
/// it appears. It runs one command and waits for it: the shell that owns
/// background runs advertises [`background_definitions`] instead.
pub(crate) fn definition() -> ToolDefinition {
    shell_definition(None)
}

/// The three definitions a shell that owns background runs advertises, as one
/// family: the start, and the two calls that reach what it started. `owner`
/// says what ends the run, because that differs by surface.
pub(crate) fn background_definitions(owner: BackgroundOwner) -> Vec<ToolDefinition> {
    vec![
        shell_definition(Some(owner)),
        output_definition(),
        kill_definition(),
    ]
}

/// Whether `name` is a `shell` call that starts a background run: the start
/// itself, with `run_in_background: true`. Read before the call runs, so a
/// surface can note the start without cloning its arguments.
pub(crate) fn starts_background(name: &str, arguments: &Value) -> bool {
    operation(name) == Some(Operation::Run)
        && arguments.get("run_in_background") == Some(&Value::Bool(true))
}

/// Whether `name` is one of the two calls that reach a background run. They
/// are the family's readers, never the start: the start is what makes the run
/// they reach, so it is advertised from the beginning.
pub(crate) fn reaches_background(name: &str) -> bool {
    matches!(operation(name), Some(Operation::Output | Operation::Kill))
}

/// The `shell` definition, with the background start added only for a shell
/// that can serve one. One builder for both, so the two can differ in exactly
/// the parameter and the sentence that describe the difference.
fn shell_definition(background: Option<BackgroundOwner>) -> ToolDefinition {
    let mut parameters = crate::agent::tools::lean::shell_parameters();
    let mut description = crate::agent::tools::lean::SHELL.to_owned();
    if let Some(owner) = background {
        parameters["properties"]["run_in_background"] = json!({
            "type": "boolean",
            "description": "Start the command and return at once with an id; you are told when it exits."
        });
        description.push_str(&format!(
            " Set `run_in_background` to start it detached and return an id; it runs until it \
             ends, you kill it, or {}. A foreground call that hits its `timeout` is moved to \
             the background and answers with its id.",
            owner.ending(),
        ));
        if owner == BackgroundOwner::Session {
            description.push_str(crate::agent::tools::lean::SHELL_SESSION);
        }
    }
    ToolDefinition {
        name: "shell".into(),
        description,
        read_only: false,
        parameters,
        effect: ToolEffect {
            database_data: false,
            external_side_effect: true,
            requires_approval: true,
            local_state: LocalStateEffect::WriteWorkspace,
        },
        concurrency: ToolConcurrency::Serial,
        completion: Some("workspace command finished".into()),
    }
}

/// `shell_output`: the read that reaches a background run this shell started.
/// Read-shaped and unapproved — it carries no authority the start did not
/// already carry — and its first read answers at once; a repeated read of a
/// quiet run waits for a change. The loop reports the exit.
fn output_definition() -> ToolDefinition {
    ToolDefinition {
        name: "shell_output".into(),
        description: "Reads a background shell's output since the last read and its status. It \
                      returns at once; repeated reads of a quiet run wait for a change. A live \
                      run says so; you are told when it exits. Output is capped at 30,000 \
                      characters per read."
            .into(),
        read_only: true,
        parameters: json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "The id a background start returned."
                }
            },
            "required": ["id"],
            "additionalProperties": false
        }),
        effect: reader_effect(),
        // Reads of separate background runs are independent even when they
        // wait, so a turn that reads several runs overlaps them. The kill
        // still serialises, because it ends the run it names.
        concurrency: ToolConcurrency::Concurrent,
        completion: Some("background shell read".into()),
    }
}

/// `shell_kill`: the call that ends a background run and its whole process
/// group. Read-shaped for the same reason the read is — it reaches only a run
/// this shell started.
fn kill_definition() -> ToolDefinition {
    ToolDefinition {
        name: "shell_kill".into(),
        description:
            "Kills a background shell started with run_in_background and its whole process \
            group, and returns its final status. The run is released."
                .into(),
        read_only: true,
        parameters: json!({
            "type": "object",
            "properties": {
                "id": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "The id a background start returned."
                }
            },
            "required": ["id"],
            "additionalProperties": false
        }),
        effect: reader_effect(),
        concurrency: ToolConcurrency::Serial,
        completion: Some("background shell killed".into()),
    }
}

/// The two readers' effect: no data, no external side effect, no approval, no
/// local state. Nothing here is new authority — the start is where the lane
/// was decided and, where the lane asks, where the user answered.
fn reader_effect() -> ToolEffect {
    ToolEffect {
        database_data: false,
        external_side_effect: false,
        requires_approval: false,
        local_state: LocalStateEffect::None,
    }
}

/// Whether a model's tool name is the shell itself: the canonical name is
/// `shell` (a model may say `bash`). The one spelling rule every surface that
/// dispatches the foreground shell shares; the background family's names are
/// [`operation`]'s.
pub(crate) fn is_shell_name(name: &str) -> bool {
    crate::agent::tools::workspace::canonical_name(name) == "shell"
}
