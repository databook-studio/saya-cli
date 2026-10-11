//! One native coding application owner shared by terminal and tool adapters.
mod approval;
pub(crate) mod checks;
pub(crate) mod cli;
mod cli_mutations;
mod command;
mod doctor;
mod doctor_extensions;
mod doctor_probe;
mod worktrees;
pub use cli::dispatch as dispatch_coding;
mod controls;
mod create;
mod delegate;
mod export;
mod observations;
mod operations;
mod parent;
mod project_context;
pub use delegate::CodingTask;
mod conflicts;
mod context;
mod continuation;
mod host;
mod integrate;
mod notes;
mod proposal;
mod provider_binding;
mod request;
pub use proposal::CodingApprovalRequest;
mod review;
mod review_checks;
mod review_diff;
pub use review::CodingReview;
mod extension_observer;
mod extensions;
mod runtime;
mod skills;
pub use approval::CodingAuthorization;
pub use host::CodingApplication;
pub use request::CodingJobRequest;
pub use runtime::NativeCodingJob;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CodingError {
    #[error("this host could not prove isolated command confinement; no process was started")]
    ConfinementUnavailable,
    #[error("candidate changed after review; review and check it again before applying")]
    StaleReview,
    #[error("successful configured checks are required before apply-back")]
    ChecksRequired,
    #[error("native coding is disabled; enable coding.enabled in trusted configuration")]
    Disabled,
    #[error("coding request is invalid or exceeds its bound")]
    InvalidRequest,
    #[error("fresh approval for this exact coding job and scope is required")]
    ApprovalRequired,
    #[error("coding state must be private, canonical, and outside the repository")]
    StateDirectory,
    #[error("coding job is not owned by this application or has no active runtime")]
    NotOwned,
    #[error("coding work is already running")]
    Busy,
    #[error(
        "overlap retained for candidate {candidate}; inspect with coding conflicts {job} {candidate}"
    )]
    ConflictRetained { job: String, candidate: String },
    #[error("native coding operation failed")]
    Owner(#[source] Box<dyn std::error::Error + Send + Sync>),
}
impl CodingError {
    pub(crate) fn owner(error: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::Owner(Box::new(error))
    }
}
