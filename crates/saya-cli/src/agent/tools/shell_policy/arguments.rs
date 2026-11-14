//! The one validation pass a shell call shares: a refusal a call can earn
//! before anything spawns is typed here, against exactly the bounds
//! [`schema`](super::schema) advertises — the same numbers, so a call the
//! model can read is a call the shell accepts.
//!
//! The three calls of the family are validated apart because their shapes are
//! apart: a start takes a command and the flag that detaches it, and each
//! reader takes the id the start returned.

use saya_agent::ToolError;
use serde_json::Value;

use super::schema::{MAX_COMMAND_BYTES, MAX_TIMEOUT_MILLIS, MAX_WORKDIR_BYTES};

/// Validates one `shell` call's arguments against the advertised schema.
/// `background` is whether this shell advertises the background start: a shell
/// without one refuses the parameter rather than accepting a flag it would
/// ignore.
pub(crate) fn validate(arguments: &Value, background: bool) -> Result<(), ToolError> {
    let object = arguments.as_object().ok_or(ToolError::ArgumentsNotObject)?;
    if object.keys().any(|key| {
        !matches!(
            key.as_str(),
            "command" | "workdir" | "timeout" | "description" | "sandbox" | "run_in_background"
        )
    }) {
        return Err(ToolError::UnsupportedProperty);
    }
    match object.get("command").and_then(Value::as_str) {
        Some("") => return Err(ToolError::UnsupportedProperty),
        Some(command) if command.len() > MAX_COMMAND_BYTES => {
            return Err(ToolError::ShellCommandTooLong {
                bytes: command.len(),
                ceiling: MAX_COMMAND_BYTES,
            });
        }
        Some(_) => {}
        None => return Err(ToolError::UnsupportedProperty),
    }
    if object.get("workdir").is_some_and(|value| {
        value
            .as_str()
            .is_none_or(|path| path.len() > MAX_WORKDIR_BYTES || path.contains('\0'))
    }) {
        return Err(ToolError::PathNotString);
    }
    if let Some(value) = object.get("timeout") {
        let millis = value.as_u64().ok_or(ToolError::UnsupportedProperty)?;
        if millis == 0 || millis > MAX_TIMEOUT_MILLIS {
            return Err(ToolError::UnsupportedProperty);
        }
    }
    if object
        .get("description")
        .is_some_and(|value| value.as_str().is_none_or(|text| text.len() > 128))
    {
        return Err(ToolError::UnsupportedProperty);
    }
    if object
        .get("sandbox")
        .is_some_and(|value| value.as_bool().is_none())
    {
        return Err(ToolError::UnsupportedProperty);
    }
    match object.get("run_in_background") {
        // A shell that holds no background run refuses the parameter: the
        // flag is never accepted and quietly dropped.
        Some(_) if !background => return Err(ToolError::UnsupportedProperty),
        Some(value) if value.as_bool().is_none() => return Err(ToolError::UnsupportedProperty),
        // A background run is ended by `shell_kill` or by the job, so a
        // timeout would be a deadline nothing honours.
        Some(Value::Bool(true)) if object.contains_key("timeout") => {
            return Err(ToolError::ShellTimeoutWithBackground);
        }
        _ => {}
    }
    Ok(())
}

/// Validates one `shell_output` call: an id, and nothing else.
pub(crate) fn validate_output(arguments: &Value) -> Result<(), ToolError> {
    let object = arguments.as_object().ok_or(ToolError::ArgumentsNotObject)?;
    if object.keys().any(|key| key != "id") {
        return Err(ToolError::UnsupportedProperty);
    }
    background_id(object.get("id")).map(|_| ())
}

/// Validates one `shell_kill` call: an id, and nothing else.
pub(crate) fn validate_kill(arguments: &Value) -> Result<(), ToolError> {
    let object = arguments.as_object().ok_or(ToolError::ArgumentsNotObject)?;
    if object.keys().any(|key| key != "id") {
        return Err(ToolError::UnsupportedProperty);
    }
    background_id(object.get("id")).map(|_| ())
}

/// The one id shape both readers take: a positive integer. Ids start at one,
/// so zero is a typo rather than a run.
pub(crate) fn background_id(value: Option<&Value>) -> Result<u64, ToolError> {
    match value.and_then(Value::as_u64) {
        Some(id) if id > 0 => Ok(id),
        _ => Err(ToolError::UnsupportedProperty),
    }
}
