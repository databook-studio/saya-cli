use std::{collections::BTreeSet, io, io::Write};

use saya_agent::{AgentLimits, AgentRequest, AnsweringLimits};
use saya_types::{MAX_NAME_CHARS, is_name_shaped};

use super::{
    ChildSpecError, MAX_CHILD_PROFILE_NAMES, MAX_CHILD_REQUEST_BYTES, MAX_CHILD_REQUEST_ITEMS,
    MAX_CHILD_REQUEST_MODEL_CHARS,
};

pub(super) fn validate_agent_limits(
    agent: AgentLimits,
    answering: AnsweringLimits,
    aggregate_uncapped: bool,
) -> Result<(), ChildSpecError> {
    if agent.max_continuations.is_none() {
        return Err(ChildSpecError::UnboundedLimits);
    }
    // Reported tokens are deliberately absent by default: the job's progress
    // guards, not a token count, are what end a long run. A child still
    // requires the count aggregates that bound its loop.
    let aggregate = [
        agent.max_turns.is_some(),
        agent.max_tool_calls.is_some(),
        answering.max_requests.is_some(),
        answering.max_tool_calls.is_some(),
    ];
    if aggregate_uncapped && aggregate.into_iter().any(|set| set) {
        return Err(ChildSpecError::UnexpectedAggregateLimits);
    }
    if !aggregate_uncapped && aggregate.into_iter().any(|set| !set) {
        return Err(ChildSpecError::UnboundedLimits);
    }
    Ok(())
}

pub(super) fn validate_request(
    request: &AgentRequest,
    context_byte_budget: usize,
) -> Result<(), ChildSpecError> {
    if request
        .model
        .chars()
        .take(MAX_CHILD_REQUEST_MODEL_CHARS + 1)
        .count()
        > MAX_CHILD_REQUEST_MODEL_CHARS
    {
        return Err(ChildSpecError::ModelNameTooLong);
    }
    if request
        .history
        .len()
        .saturating_add(request.context_blocks.len())
        > MAX_CHILD_REQUEST_ITEMS
    {
        return Err(ChildSpecError::TooManyRequestItems);
    }
    if request.profile_names.len() > MAX_CHILD_PROFILE_NAMES {
        return Err(ChildSpecError::TooManyProfileNames);
    }
    let mut seen = BTreeSet::new();
    for profile in &request.profile_names {
        if profile.chars().take(MAX_NAME_CHARS + 1).count() > MAX_NAME_CHARS
            || !is_name_shaped(profile)
            || !seen.insert(profile.as_str())
        {
            return Err(ChildSpecError::InvalidProfileNames);
        }
    }
    let maximum = MAX_CHILD_REQUEST_BYTES.min(context_byte_budget);
    measure_request(request, maximum)
}

/// The trusted owner projection is outside the caller's public 64 KiB request
/// contract, but the assembled request must still fit the configured context
/// budget before a provider can receive it.
pub(super) fn validate_assembled_context(
    request: &AgentRequest,
    context_byte_budget: usize,
) -> Result<(), ChildSpecError> {
    measure_request(request, context_byte_budget)
}

fn measure_request(request: &AgentRequest, maximum: usize) -> Result<(), ChildSpecError> {
    let mut writer = BoundedWriter::new(maximum);
    match serde_json::to_writer(&mut writer, request) {
        Ok(()) => Ok(()),
        Err(_) if writer.exceeded => Err(ChildSpecError::RequestTooLarge),
        Err(_) => Err(ChildSpecError::InvalidRequest),
    }
}

struct BoundedWriter {
    written: usize,
    maximum: usize,
    exceeded: bool,
}

impl BoundedWriter {
    fn new(maximum: usize) -> Self {
        Self {
            written: 0,
            maximum,
            exceeded: false,
        }
    }
}

impl Write for BoundedWriter {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        let remaining = self.maximum.saturating_sub(self.written);
        let accepted = input.len().min(remaining);
        self.written += accepted;
        if accepted != input.len() {
            self.exceeded = true;
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "child request byte limit exceeded",
            ));
        }
        Ok(accepted)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
