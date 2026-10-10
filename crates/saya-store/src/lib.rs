//! Redacted session storage contracts for SAYA CLI.

mod audit_store;
mod bounded;
mod checkpoints;
mod coding;
mod continuation;
mod contracts;
mod error;
mod filesystem;
mod history;
mod investigations;
mod knowledge_items;
mod migration;
mod private_file;
mod project_preferences;
mod redaction;
mod replace;
mod runs;
mod schema_store;
mod session;
mod session_journal;
mod session_lifecycle;
mod session_store;
mod sqlite;
mod sqlite_support;
mod state_contracts;

pub use checkpoints::{CheckpointStoreError, NewCheckpoint, RecoveryReport, ScreenRefusal};
pub use coding::{
    CheckoutOperationPage, CodingApprovalClosure, CodingApprovalConsumptionAppend,
    CodingApprovalConsumptionContext, CodingApprovalConsumptionReceipt, CodingApprovalRequestPage,
    CodingApprovalRequestRecord, CodingBudgetPolicyAppend, CodingBudgetReservationPage,
    CodingBudgetReservationRecord, CodingChildPage, CodingChildRecord, CodingEventAppend,
    CodingEventPage, CodingJobRecord, MAX_CHECKOUT_OPERATION_INTENTS_PER_JOB,
    MAX_CHECKOUT_RECORDS_PER_JOB, MAX_CODING_EVENT_PAGE_SIZE, ManagedCheckoutPage,
    ManagedCheckoutRecord, NewCodingChild, NewCodingJob, StoredBudgetPolicy,
};
pub use continuation::StoredContinuation;
pub use contracts::{ForgetReason, MAX_PREFERENCE_VALUE_BYTES, PreferenceStore};
pub use error::StoreError;
pub use filesystem::{FsSessionStore, MAX_SESSION_BYTES};
pub use history::{
    MAX_SESSION_HISTORY_PAGE_SIZE, SessionHistoryCursor, SessionHistoryLimit, SessionHistoryPage,
};
pub use investigations::{
    InvestigationListIssue, InvestigationPage, InvestigationRepository, InvestigationSummary,
    LocalBinding, MAX_BINDING_BYTES, MAX_DOCUMENTS, MAX_LIST_PAGE,
};
pub use knowledge_items::{
    BatchItemOutcome, BatchOutcome, CleanupState, ForgetOutcome, KnowledgeCursor, KnowledgeItem,
    KnowledgeItemRequest, KnowledgeItemStore, KnowledgeItemsQuery, KnowledgeObjectsQuery,
    KnowledgePage, KnowledgeStoreError, MAX_KNOWLEDGE_ITEM_BYTES, MAX_KNOWLEDGE_PAGE_SIZE,
    MAX_PENDING_BATCH_ITEMS, MAX_SCHEMA_BINDING_BYTES, NewKnowledgeItem, knowledge_item_id_for,
    validate_new_item,
};
pub use project_preferences::ProjectPreferenceStore;
pub use redaction::redact;
pub use runs::{
    NewRun, RunBudgets, RunCapabilityFlags, RunRecord, RunStatus, RunStepRecord, RunStepStatus,
    RunStore, RunSummary, RunUsage,
};
pub use session::{
    ParentLink, RedactedMessage, RedactedSession, RedactedToolMetadata, RedactedToolResultShape,
    RedactedTurn, SESSION_VERSION,
};
pub use session_journal::{
    BypassSource, GrantSource, JournalEvent, MAX_JOURNAL_BYTES, SessionJournal,
};
pub use session_lifecycle::{
    DeleteStatus, SESSION_WARN_BYTES, SessionCapacity, SessionCapacityStatus, SuccessorOutcome,
};
pub use session_store::{SessionHistoryQuery, SessionStore, SessionSummary};
pub use sqlite::{OPEN_BUSY_CEILING, SqliteStateStore};
pub use sqlite_support::state_sidecar_path;
pub use state_contracts::{
    AuditEntry, AuditOperation, AuditRecord, AuditStatus, AuditStore, CachedSchema, SCHEMA_VERSION,
    SchemaCacheEntry, SchemaStore,
};
