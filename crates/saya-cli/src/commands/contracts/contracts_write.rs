//! Mutating `contracts` commands: `remember`, `review`, `forget`. Each resolves
//! its arguments, calls a `crate::contracts` write operation, and emits a
//! `ContractChanged` event. A write against an unavailable store is a genuine
//! failure and exits non-zero — the user asked for something that did not happen.

use super::contracts_remember_schema::{
    SchemaCheck, fingerprint_of, refuse_unknown, resolved_against,
};
use super::{
    ArgMessage, EXIT_CONTRACT_ERROR, arg_failure, cached_schema, op_failure, parse_claim_id,
    unobserved_fingerprint,
};
use crate::cli::{ClaimKindArg, ForgetReasonArg};
use crate::commands::output::{emit, failure_message};
use crate::contracts::args::{ReviewDecision, build_payload, parse_qualified, review_decision};
use crate::contracts::{RememberOutcome, confirm, forget, reject, remember as remember_op};
use crate::render::{RenderFormat, TerminalEvent};
use saya_store::{ForgetReason, SqliteStateStore};
use saya_types::{
    ClaimStatus, DatabaseObjectKind, DatabaseObjectRef, KnowledgeState, ProfileIdentity,
};

/// What the user asked to remember: the qualified object, the kind, and the
/// claim value (with its optional column). Borrows the parsed strings so a
/// `remember` call allocates nothing it does not have to.
pub(super) struct RememberRequest<'a> {
    pub table: &'a str,
    pub kind: ClaimKindArg,
    pub value: &'a str,
    pub column: Option<&'a str>,
}

/// Where it is being remembered: the store, the render format, and the
/// resolved profile (name + opaque identity). The name is what renders; the
/// identity is what the schema cache and the stored `DatabaseObjectRef` are
/// keyed by.
pub(super) struct RememberContext<'a> {
    pub store: &'a SqliteStateStore,
    pub format: RenderFormat,
    pub profile_name: &'a str,
    pub identity: &'a ProfileIdentity,
}

pub(super) async fn remember(
    request: RememberRequest<'_>,
    context: RememberContext<'_>,
) -> Result<i32, Box<dyn std::error::Error>> {
    let RememberRequest {
        table,
        kind,
        value,
        column,
    } = request;
    let RememberContext {
        store,
        format,
        profile_name,
        identity,
    } = context;
    let qualified = match parse_qualified(table) {
        Ok(q) => q,
        Err(_) => return arg_failure(ArgMessage::MalformedTable, format),
    };
    let payload = match build_payload(kind, value, column) {
        Ok(payload) => payload,
        Err(_) => return arg_failure(ArgMessage::BadValue, format),
    };
    let object = match DatabaseObjectRef::new(
        identity.clone(),
        &qualified.catalog,
        &qualified.schema,
        &qualified.object,
        DatabaseObjectKind::Table,
    ) {
        Ok(object) => object,
        Err(_) => return arg_failure(ArgMessage::MalformedTable, format),
    };
    // `remember` classifies against the cached schema — the same source
    // `list`/`show` use — so a claim made right after a `connection schema
    // --refresh` stores the real digest and reads `current`, and a claim keyed
    // to an object the cache lacks is refused here, not stored silently and
    // marked stale only at the next refresh. No cache, or an empty cached tree
    // (no real schema information), keeps the original behaviour: the
    // unobserved sentinel and no validation. The check lives in
    // `contracts_remember_schema`; this is the call site. The structural
    // dependency a later drift checks is the `SchemaBinding` the `remember` op
    // derives from `(slot, payload)`, so the column snapshots the legacy
    // `ProposeClaim` carried are no longer computed here.
    let cached = cached_schema(store, identity).await;
    let fingerprint = match resolved_against(&cached, &object) {
        SchemaCheck::Found(table) => fingerprint_of(table),
        SchemaCheck::Absent => return refuse_unknown(&object, profile_name, format),
        SchemaCheck::NoSchema => unobserved_fingerprint(),
    };
    let outcome = match remember_op(store, &object, &payload, fingerprint).await {
        Ok(outcome) => outcome,
        Err(error) => return op_failure(error, format),
    };
    // A duplicate is not an error: pass the existing item's real status through
    // so a duplicate of a forgotten claim reads as forgotten, not as success.
    // The `ki-…` id is the same in either arm — the stored row, or the
    // pre-existing one a duplicate names — and it stays on the event for JSON
    // and NDJSON consumers even though the text confirmation no longer shows it.
    let (claim_id, action, status) = match outcome {
        RememberOutcome::Stored { id } => (id, "remembered", ClaimStatus::Confirmed),
        RememberOutcome::Duplicate { id, state } => (id, "duplicate", status_from_state(state)),
    };
    emit(
        TerminalEvent::ContractRemembered {
            claim_id: claim_id.as_str().to_string(),
            object: format!(
                "{}.{}.{}",
                qualified.catalog, qualified.schema, qualified.object
            ),
            kind: kind.as_str().to_string(),
            value: value.to_string(),
            column: column.map(str::to_string),
            action: action.into(),
            status: status.as_str().into(),
        },
        format,
    );
    Ok(0)
}

/// The rendered status word for a duplicate's persisted [`KnowledgeState`]: an
/// active duplicate is `confirmed`, a pending one `candidate`, a dismissed one
/// `forgotten` (the case `remember-after-forget` names — a re-remember of a
/// forgotten tombstone reports `forgotten`, not success). Mirrors
/// `contracts::view::status_from_state`'s mapping but returns the
/// `ClaimStatus` the `ContractChanged` event carries; kept here because the
/// write path is the only consumer of the `Duplicate` arm's state.
fn status_from_state(state: KnowledgeState) -> ClaimStatus {
    match state {
        KnowledgeState::Active => ClaimStatus::Confirmed,
        KnowledgeState::Pending => ClaimStatus::Candidate,
        // A dismissed duplicate is a forgotten/rejected value; "forgotten"
        // matches the `duplicate of {id} — previously forgotten` render the
        // `changed` shaper emits for a forgotten duplicate.
        KnowledgeState::Dismissed => ClaimStatus::Forgotten,
        // `KnowledgeState` is `#[non_exhaustive]`; a future variant the render
        // layer does not know about fails closed to a non-success word rather
        // than guess an authority it does not have.
        _ => ClaimStatus::Forgotten,
    }
}

pub(super) async fn review(
    store: &SqliteStateStore,
    format: RenderFormat,
    claim_id: &str,
    do_confirm: bool,
    do_reject: bool,
) -> Result<i32, Box<dyn std::error::Error>> {
    let id = match parse_claim_id(claim_id) {
        Ok(id) => id,
        Err(message) => return failure_message(EXIT_CONTRACT_ERROR, message, format),
    };
    let decision = match review_decision(do_confirm, do_reject) {
        Ok(decision) => decision,
        Err(_) => return arg_failure(ArgMessage::AmbiguousReview, format),
    };
    let claim = match decision {
        ReviewDecision::Confirm => confirm(store, &id).await,
        ReviewDecision::Reject => reject(store, &id).await,
    };
    let claim = match claim {
        Ok(claim) => claim,
        Err(error) => return op_failure(error, format),
    };
    let (action, status) = match claim.status {
        ClaimStatus::Confirmed => ("confirmed", "confirmed"),
        ClaimStatus::Rejected => ("rejected", "rejected"),
        other => ("reviewed", other.as_str()),
    };
    emit(
        TerminalEvent::ContractChanged {
            claim_id: claim.id.as_str().to_string(),
            action: action.into(),
            status: status.into(),
        },
        format,
    );
    Ok(0)
}

pub(super) async fn forget_claim(
    store: &SqliteStateStore,
    format: RenderFormat,
    claim_id: &str,
    reason: ForgetReasonArg,
) -> Result<i32, Box<dyn std::error::Error>> {
    let id = match parse_claim_id(claim_id) {
        Ok(id) => id,
        Err(message) => return failure_message(EXIT_CONTRACT_ERROR, message, format),
    };
    if let Err(error) = forget(store, &id, forget_reason(reason)).await {
        return op_failure(error, format);
    }
    emit(
        TerminalEvent::ContractChanged {
            claim_id: id.as_str().to_string(),
            action: "forgotten".into(),
            status: "forgotten".into(),
        },
        format,
    );
    Ok(0)
}

fn forget_reason(arg: ForgetReasonArg) -> ForgetReason {
    match arg {
        ForgetReasonArg::UserRequest => ForgetReason::UserRequest,
        ForgetReasonArg::Incorrect => ForgetReason::Incorrect,
        ForgetReasonArg::Obsolete => ForgetReason::Obsolete,
        ForgetReasonArg::Privacy => ForgetReason::Privacy,
    }
}
