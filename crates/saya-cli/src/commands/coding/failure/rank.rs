//! The fixed severity rank of every coding failure class.

use saya_types::CodingFailureClass as Class;

/// The fixed severity rank of every `CodingFailureClass`: the HIGHER the rank,
/// the more it explains what an operator must act on, and the primary the job
/// reports. The latch keeps the highest-ranked class it has seen, so the same
/// SET of failures always yields the same primary and secondary whatever order
/// the failures arrive in (ledger row 37). From most to least severe:
///
/// - `MandatoryPersistence`: a durable record the job must write could not be
///   written, so the job's own authority is incomplete and cannot be trusted.
/// - `EffectsUncertain`: a child's effect was never settled, so W may or may
///   not hold that child's writes.
/// - `AdmissionDenied`: the job's shared budget or effect owner refused the
///   next step; a bound to raise, not a fault, and nothing was applied.
/// - `DependencyBlocked`: a child's dependency was never satisfied, so its
///   piece never ran at all.
/// - `SetupFailed`: a child could not be set up, or its channel identity did
///   not match.
/// - `ExtensionExchange`: a trusted extension's bounded protocol exchange
///   failed.
/// - `ProviderStreamEnded`: the provider's response stream ended before it
///   finished, usually a gateway or provider time limit.
/// - `ProviderError`: the provider call itself failed.
/// - `OutputLimit`: the model spent its output-token limit before answering.
/// - `InvalidToolCall`: the model produced a tool call or history the loop
///   could not use.
/// - `Unsupported`: the provider cannot supply the accounting the job requires.
/// - `LimitReached`: the agent loop reached one of its own limits.
/// - `CoordinatorStalled`: the coordinator's calls changed nothing for too long.
/// - `CoordinatorNoProgress`: the coordinator kept calling tools after none
///   could advance the job.
/// - `DiagnosticStorage`: the job's diagnostics could not be recorded; the run
///   is under-audited, not wrong.
/// - `ProgressDelivery`: progress the operator watches could not be delivered.
/// - `Cancelled`: the run was stopped by something other than the user.
/// - `WallClock` / `Cost` / `Backstop`: a stated guard ended the run: a
///   bounded end, not a fault.
/// - `NoProgress` / `Repeating`: the job stopped making progress: also a
///   bounded end.
/// - `BudgetStopped`: an older durable record's budget end.
/// - `Internal`: anything else, including owner failures: the generic class,
///   so every named cause above outranks it.
pub(super) fn class_rank(class: Class) -> u8 {
    match class {
        Class::MandatoryPersistence => 24,
        Class::EffectsUncertain => 23,
        Class::AdmissionDenied => 22,
        Class::DependencyBlocked => 21,
        Class::SetupFailed => 20,
        Class::ExtensionExchange => 19,
        Class::ProviderStreamEnded => 18,
        Class::ProviderError => 17,
        Class::OutputLimit => 16,
        Class::InvalidToolCall => 15,
        Class::Unsupported => 14,
        Class::LimitReached => 13,
        Class::CoordinatorStalled => 12,
        Class::CoordinatorNoProgress => 11,
        Class::DiagnosticStorage => 10,
        Class::ProgressDelivery => 9,
        Class::Cancelled => 8,
        Class::WallClock => 7,
        Class::Cost => 6,
        Class::Backstop => 5,
        Class::NoProgress => 4,
        Class::Repeating => 3,
        Class::BudgetStopped => 2,
        Class::Internal => 1,
    }
}
