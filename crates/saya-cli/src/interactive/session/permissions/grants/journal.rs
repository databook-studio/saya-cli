//! The failed-journal wording and the `[s]` prompt answer's one operation.

use saya_agent::{ApprovalChoice, SessionPolicy};
use saya_store::{GrantSource, SessionJournal};

/// How a failed journal write is said — the one wording every journaling
/// site renders, so the two surfaces cannot drift. The consent always
/// stands; only the audit line is missing.
pub(crate) fn journal_warning(error: &saya_store::StoreError) -> String {
    format!(
        "warning: the session journal could not record this ({error}) — the grant stands, \
         but the audit line is missing"
    )
}

/// The `[s]` answer's one operation, shared by both prompt surfaces: record
/// the answer in the session's one policy, and journal a *new* grant there —
/// before the call it allowed runs, which is the best a per-call approval
/// surface can do and the ordering that makes the record meaningful. Allow-
/// once and deny record nothing. Returns whether a new grant landed, and —
/// when the journal write failed — the one warning wording: the consent
/// stands, the audit line is missing, and saying so is never optional.
pub(crate) fn record_prompt_answer(
    policy: &SessionPolicy,
    choice: &ApprovalChoice,
    journal: Option<&SessionJournal>,
) -> (bool, Option<String>) {
    let new_grant = policy.record(choice.clone());
    if !new_grant {
        return (false, None);
    }
    let ApprovalChoice::AllowSession { token } = choice else {
        return (true, None);
    };
    let Some(journal) = journal else {
        return (true, None);
    };
    match journal.granted(token, GrantSource::Prompt) {
        Ok(()) => (true, None),
        Err(error) => (true, Some(journal_warning(&error))),
    }
}
