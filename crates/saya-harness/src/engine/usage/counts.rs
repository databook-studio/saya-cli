use super::AnsweringCountError;
use saya_agent::AnsweringObservation;
use saya_types::RunEvent;

#[cfg(test)]
#[path = "counts_tests.rs"]
mod counts_tests;

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct StepAnsweringCounts {
    step: Option<usize>,
    initialized: bool,
    fresh_initialization: bool,
    pending: bool,
    completed: bool,
    requests: u64,
    tool_calls: u64,
}

impl StepAnsweringCounts {
    pub(crate) fn from_journal(
        events: &[RunEvent],
        target: usize,
    ) -> Result<Self, AnsweringCountError> {
        let mut counts = Self::default();
        for event in events {
            match event {
                RunEvent::StepStarted { step } => match counts.step {
                    Some(active) if active == *step && !counts.completed => {}
                    Some(_) if !counts.completed => {
                        return Err(AnsweringCountError::InvalidEvidence);
                    }
                    _ => {
                        counts = Self {
                            step: Some(*step),
                            fresh_initialization: true,
                            ..Self::default()
                        };
                    }
                },
                RunEvent::StepCompleted { step } if counts.step == Some(*step) => {
                    counts.completed = true;
                }
                RunEvent::Usage {
                    tokens,
                    turns,
                    tool_calls,
                    cached_input_tokens,
                    cache_creation_input_tokens,
                    ..
                } if turns.is_some() || tool_calls.is_some() => {
                    if tokens.is_some()
                        || cached_input_tokens.is_some()
                        || cache_creation_input_tokens.is_some()
                        || counts.step.is_none()
                        || counts.completed
                    {
                        return Err(AnsweringCountError::InvalidEvidence);
                    }
                    let observation = match (*turns, *tool_calls) {
                        (Some(0), Some(0)) => AnsweringObservation::Initialized,
                        (Some(1), None) => AnsweringObservation::RequestAdmitted,
                        (None, Some(requested)) => {
                            AnsweringObservation::ToolBatchObserved { requested }
                        }
                        _ => return Err(AnsweringCountError::InvalidEvidence),
                    };
                    counts.observe(observation)?;
                }
                _ => {}
            }
        }
        if !counts.initialized {
            counts.fresh_initialization = false;
        }
        if counts.step == Some(target) {
            Ok(counts)
        } else {
            Ok(Self::default())
        }
    }

    pub(crate) fn begin_step(&mut self, step: usize) {
        if self.step != Some(step) || self.completed {
            *self = Self {
                step: Some(step),
                fresh_initialization: true,
                ..Self::default()
            };
        }
    }

    pub(crate) fn remaining(
        &self,
        step: usize,
        turns: Option<u64>,
        tool_calls: Option<u64>,
    ) -> Result<(Option<u64>, Option<u64>), AnsweringCountError> {
        if turns.is_none() && tool_calls.is_none() {
            return Ok((turns, tool_calls));
        }
        if self.step != Some(step) {
            return Ok((turns, tool_calls));
        }
        if !self.initialized && !self.fresh_initialization {
            return Err(AnsweringCountError::MissingInitialization);
        }
        if self.pending {
            return Err(AnsweringCountError::PendingBatch);
        }
        Ok((
            turns.map(|limit| limit.saturating_sub(self.requests)),
            tool_calls.map(|limit| limit.saturating_sub(self.tool_calls)),
        ))
    }

    pub(crate) fn observe(
        &mut self,
        observation: AnsweringObservation,
    ) -> Result<bool, AnsweringCountError> {
        match observation {
            AnsweringObservation::Initialized if self.initialized && !self.pending => Ok(false),
            AnsweringObservation::Initialized if self.fresh_initialization => {
                self.initialized = true;
                self.fresh_initialization = false;
                Ok(true)
            }
            AnsweringObservation::Initialized => Err(AnsweringCountError::InvalidEvidence),
            AnsweringObservation::RequestAdmitted if self.initialized && !self.pending => {
                self.requests = self.requests.saturating_add(1);
                self.pending = true;
                Ok(true)
            }
            AnsweringObservation::RequestAdmitted => Err(AnsweringCountError::InvalidEvidence),
            AnsweringObservation::ToolBatchObserved { requested } if self.pending => {
                self.tool_calls = self.tool_calls.saturating_add(requested);
                self.pending = false;
                Ok(true)
            }
            AnsweringObservation::ToolBatchObserved { .. } => {
                Err(AnsweringCountError::BatchWithoutRequest)
            }
            _ => Err(AnsweringCountError::InvalidEvidence),
        }
    }
}
