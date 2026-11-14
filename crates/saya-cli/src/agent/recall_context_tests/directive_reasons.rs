use super::*;

/// A directive claim that carries a reason renders it on the line beneath
/// the claim, plainly attached to the claim it justifies (spec:
/// claim-reasons). The `[confirmed]` marker and the claim line are unchanged;
/// the reason is an addition, not a rewording, and the `CONFIRMED_DIRECTIVE`
/// stanza is not weakened. A claim with no reason renders exactly as before —
/// no extra line.
#[tokio::test]
async fn a_directive_claim_renders_its_reason_under_the_claim_line() {
    use crate::contracts::{ContractClaim, ContractConflict, RetrievedContract};

    let identity = identity_for("analytics");
    let obj = object(&identity, "rental");
    let contract = RetrievedContract {
        object: obj.clone(),
        schema_state: crate::contracts::ContractSchemaState::Current,
        claims: vec![ContractClaim {
            id: ClaimId::parse("c-rental-time").unwrap(),
            object: obj.clone(),
            value: ClaimPayload::default_time_column(
                "return_date",
                Some("a rental only counts once it comes back"),
            )
            .unwrap(),
            source: ClaimOrigin::UserExplicit,
            status: ClaimStatus::Confirmed,
        }],
        conflicts: Vec::<ContractConflict>::new(),
        truncated: false,
        incomplete: false,
    };
    let name_of =
        std::collections::HashMap::from([(identity.as_str().to_string(), "analytics".into())]);
    let body = super::render::render_body(std::slice::from_ref(&contract), &name_of);

    // The claim line is intact and reads as the binding directive it is.
    assert!(
        body.contains("[confirmed] default_time_column  return_date\n"),
        "the claim line is unchanged: {body}"
    );
    // The reason follows, on its own line, indented under the claim and
    // labelled so it reads as the justification for the claim above it.
    assert!(
        body.contains("      because: a rental only counts once it comes back\n"),
        "the reason renders under the claim, attached: {body}"
    );
    // The directive stanza is untouched.
    assert!(body.contains(super::render::CONFIRMED_DIRECTIVE));
}

/// A claim with no reason renders no extra line — the common case, and the
/// state of every directive claim written before the field existed.
#[tokio::test]
async fn a_directive_claim_with_no_reason_renders_no_reason_line() {
    use crate::contracts::{ContractClaim, ContractConflict, RetrievedContract};

    let identity = identity_for("analytics");
    let obj = object(&identity, "rental");
    let contract = RetrievedContract {
        object: obj.clone(),
        schema_state: crate::contracts::ContractSchemaState::Current,
        claims: vec![ContractClaim {
            id: ClaimId::parse("c-rental-time").unwrap(),
            object: obj.clone(),
            value: ClaimPayload::default_time_column("return_date", None).unwrap(),
            source: ClaimOrigin::UserExplicit,
            status: ClaimStatus::Confirmed,
        }],
        conflicts: Vec::<ContractConflict>::new(),
        truncated: false,
        incomplete: false,
    };
    let name_of =
        std::collections::HashMap::from([(identity.as_str().to_string(), "analytics".into())]);
    let body = super::render::render_body(std::slice::from_ref(&contract), &name_of);
    assert!(
        body.contains("[confirmed] default_time_column  return_date\n"),
        "the claim line renders: {body}"
    );
    assert!(
        !body.contains("because:"),
        "no reason line when there is no reason: {body}"
    );
}
