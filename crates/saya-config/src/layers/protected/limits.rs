use crate::ConfigFile;
use crate::limits::{AnsweringLimitsFile, JobLimitsFile};

/// The protected sub-tables of `[limits]`: the answering surfaces' and the
/// coding surface's.
///
/// Protection is per sub-table, never for `[limits]` as a whole: these ceilings
/// are security-critical (a project layer that could raise them would spend the
/// user's money and time), while `[limits.run]` follows the `[jobs]` ceilings,
/// which a project layer may set. Keeping the decision here,
/// sub-table by sub-table, is what stops one of them silently inheriting the
/// other's answer.
pub(super) struct ProtectedLimits {
    session: AnsweringLimitsFile,
    ask: AnsweringLimitsFile,
    job: JobLimitsFile,
}

pub(super) fn snapshot(file: &ConfigFile) -> ProtectedLimits {
    ProtectedLimits {
        session: file.limits.session.clone(),
        ask: file.limits.ask.clone(),
        job: file.limits.job.clone(),
    }
}

/// Restores the protected sub-tables to their trusted values and returns the
/// dotted names the project layer tried to override. Named per sub-table rather
/// than per key: the whole table reverts, and `limits.session` is what tells a
/// user which block to fix.
pub(super) fn revert(file: &mut ConfigFile, trusted: &ProtectedLimits) -> Vec<String> {
    let mut ignored = Vec::new();
    if file.limits.session != trusted.session {
        file.limits.session = trusted.session.clone();
        ignored.push("limits.session".into());
    }
    if file.limits.ask != trusted.ask {
        file.limits.ask = trusted.ask.clone();
        ignored.push("limits.ask".into());
    }
    if file.limits.job != trusted.job {
        file.limits.job = trusted.job.clone();
        ignored.push("limits.job".into());
    }
    ignored
}
