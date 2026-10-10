//! Agent protocol contracts — the request/response shapes, event stream, tool
//! definitions, and errors that cross the `saya-agent` → `saya-cli` boundary.
//! Declarations and the public surface live here; the types are split into
//! per-concern submodules.

mod approval;
mod budget;
mod builders;
mod chat;
mod coding_admission;
mod coding_usage;
mod context;
mod error;
mod event;
mod knowledge;
mod learning;
mod recovery;
mod session_policy;
mod tool;
mod usage;

#[cfg(test)]
mod chat_wire_tests;
#[cfg(test)]
mod knowledge_reasoning_tests;
#[cfg(test)]
mod session_policy_tests;
#[cfg(test)]
mod tool_effect_tests;

pub use approval::{
    AgentMode, AgentModeParseError, AllowReadOnlyApproval, ApprovalDecider, read_only_permits,
};
pub use budget::{AnsweringObservation, BudgetDimension};
pub use chat::{
    AgentRequest, ChatMessage, ChatRequest, ChatResponse, ContextBlock, ReasoningEffort,
    ResponseFormat, ToolCall, ToolMetadata, ToolResultShape,
};
pub use coding_admission::{
    EffectAdmission, EffectAdmissionError, EffectOutcome, EffectOwnerFailure, EffectPermit,
    EffectSettlement,
};
pub use coding_usage::{EffectCallKind, PhysicalAttempt};
pub use context::{CONTEXT_COMPACT_PERCENT, CONTEXT_WARN_PERCENT, context_utilisation_percent};
pub use error::{ProviderError, ToolError};
pub use event::{AgentEvent, ProviderContinuationPhase};
pub use knowledge::{
    KnowledgeOutcome, OverrideFindingDto, ProposedClaimDto, SuppliedClaimDto, SuppliedContractDto,
};
pub use learning::LearningSkipReason;
pub use recovery::{ProviderRecoveryPhase, ProviderRecoveryReason};
pub use session_policy::{ApprovalChoice, ApprovalDecision, SessionGrants, SessionPolicy};
pub use tool::{
    LocalStateEffect, ToolConcurrency, ToolDefinition, ToolEffect, ToolExecutionContext,
    ToolExecutor,
};
pub use usage::UsageCall;
