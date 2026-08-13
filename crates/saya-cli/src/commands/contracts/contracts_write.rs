//! Mutating `contracts` commands: `remember`, `review`, `forget`. Each resolves
//! its arguments, calls a `crate::contracts` write operation, and emits a
//! `ContractChanged` event. A write against an unavailable store is a genuine
//! failure and exits non-zero — the user asked for something that did not happen.

use super::{
    ArgMessage, EXIT_CONTRACT_ERROR, arg_failure, op_failure, parse_claim_id,
    unobserved_fingerprint,
};
use crate::cli::{ClaimKindArg, ForgetReasonArg};
use crate::commands::output::{emit, failure_message};
use crate::contracts::args::{ReviewDecision, build_payload, parse_qualified, review_decision};
use crate::contracts::{confirm, forget, propose, reject};
use crate::render::{RenderFormat, TerminalEvent};
use saya_store::{ForgetReason, ProposeClaim, ProposeOutcome, SqliteStateStore};
use saya_types::{
    ClaimOrigin, ClaimStatus, DatabaseObjectKind, DatabaseObjectRef, ProfileIdentity,
};

pub(super) async fn remember(
    store: &SqliteStateStore,
    format: RenderFormat,
    identity: &ProfileIdentity,
    table: &str,
    kind: ClaimKindArg,
    value: &str,
    column: Option<&str>,
) -> Result<i32, Box<dyn std::error::Error>> {
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
    let request = ProposeClaim {
        object,
        fingerprint: unobserved_fingerprint(),
        payload,
        origin: ClaimOrigin::UserExplicit,
        initial_status: ClaimStatus::Confirmed,
        evidence: None,
    };
    let outcome = match propose(store, request).await {
        Ok(outcome) => outcome,
        Err(error) => return op_failure(error, format),
    };
    // A duplicate is not an error: pass the existing claim's real status through
    // so a duplicate of a forgotten claim reads as forgotten, not as success.
    let (claim_id, action, status) = match outcome {
        ProposeOutcome::Stored(id) => (id, "remembered", ClaimStatus::Confirmed),
        ProposeOutcome::Duplicate { id, status } => (id, "duplicate", status),
    };
    emit(
        TerminalEvent::ContractChanged {
            claim_id: claim_id.as_str().to_string(),
            action: action.into(),
            status: status.as_str().into(),
        },
        format,
    );
    Ok(0)
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
