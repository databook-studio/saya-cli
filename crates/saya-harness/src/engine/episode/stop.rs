use saya_agent::{BudgetDimension, CancellationReason};
use saya_types::PauseReason;

use crate::engine::{sink::EngineEventSink, state::RunState, transitions::TransitionEvent};

use super::{EpisodeDriver, EpisodeError};

impl<'a> EpisodeDriver<'a> {
    pub(super) async fn settle_stop(
        &self,
        sink: &EngineEventSink,
        step: usize,
        reason: CancellationReason,
    ) -> Result<(), EpisodeError> {
        match reason {
            CancellationReason::User => {
                let diagnostic = sink.take_diagnostic();
                match sink.record(TransitionEvent::Cancel).await {
                    Ok(_) => match diagnostic {
                        Some(diagnostic) => {
                            Err(EpisodeError::CancelledDiagnostic { step, diagnostic })
                        }
                        None => Err(EpisodeError::Cancelled { step }),
                    },
                    Err(source) => match diagnostic {
                        Some(diagnostic) => Err(EpisodeError::CancelPersistenceFailed {
                            step,
                            diagnostic,
                            transition: std::sync::Arc::new(source),
                        }),
                        None => Err(EpisodeError::Transition { source }),
                    },
                }
            }
            CancellationReason::Budget(dimension) => {
                let reason = if dimension == BudgetDimension::ElapsedTime {
                    PauseReason::WallClockExceeded
                } else {
                    PauseReason::BudgetExhausted
                };
                let prior_diagnostic = sink.take_diagnostic();
                let transition = sink.record(TransitionEvent::Pause(reason)).await;
                let diagnostic = prior_diagnostic.or_else(|| sink.take_diagnostic());
                match transition {
                    Ok(_) => match diagnostic {
                        Some(diagnostic) => Err(EpisodeError::BudgetStoppedDiagnostic {
                            step,
                            reason,
                            diagnostic,
                        }),
                        None => Err(EpisodeError::BudgetStopped { step, reason }),
                    },
                    Err(source) => match diagnostic {
                        Some(diagnostic) => Err(EpisodeError::BudgetPersistenceFailed {
                            step,
                            reason,
                            diagnostic,
                            transition: std::sync::Arc::new(source),
                        }),
                        None => Err(EpisodeError::Transition { source }),
                    },
                }
            }
            CancellationReason::OwnerFailure => {
                let diagnostic = sink.take_diagnostic();
                if diagnostic.as_deref().is_some_and(|error| {
                    matches!(error, crate::engine::sink::EngineSinkError::Store { .. })
                }) && sink.state() == RunState::Executing
                    && let Err(error) = sink
                        .record(TransitionEvent::Pause(PauseReason::StoreUnavailable))
                        .await
                {
                    sink.hold_diagnostic(error);
                }
                match diagnostic.or_else(|| sink.take_diagnostic()) {
                    Some(diagnostic) => {
                        Err(EpisodeError::OwnerFailureDiagnostic { step, diagnostic })
                    }
                    None => Err(EpisodeError::OwnerFailure { step }),
                }
            }
        }
    }
}
