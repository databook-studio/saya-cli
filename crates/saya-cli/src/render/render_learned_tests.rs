use super::*;
use saya_types::ClaimId;

fn claim(kind: &str, value: &str, column: Option<&str>, status: ClaimStatus) -> ProposedClaimDto {
    ProposedClaimDto {
        claim_id: ClaimId::parse(&"a".repeat(64)).unwrap(),
        profile: "analytics".into(),
        object: "pagila.public.staff".into(),
        kind: kind.into(),
        value: value.into(),
        column: column.map(str::to_string),
        status,
    }
}

#[test]
fn a_user_stated_fact_reads_as_learned_and_names_the_object() {
    let line = knowledge_learned_text(&claim(
        "table_alias",
        "account managers",
        None,
        ClaimStatus::Confirmed,
    ));
    assert_eq!(
        line, "memory learned · the alias \"account managers\" for pagila.public.staff\n",
        "the line names the fact and the object"
    );
}

#[test]
fn an_inferred_fact_says_it_is_unconfirmed_and_where_to_review_it() {
    let line = knowledge_learned_text(&claim(
        "default_time_column",
        "return_date",
        None,
        ClaimStatus::Candidate,
    ));
    assert!(
        line.contains("memory noted ·"),
        "an inference is noted, not learned: {line}"
    );
    assert!(
        line.contains("unconfirmed") && line.contains("/queue"),
        "an inference says it is waiting and where to act: {line}"
    );
}

/// The point of the whole change: nothing the user did not ask for hands them a
/// hash to deal with.
#[test]
fn no_claim_id_reaches_the_line_in_either_status() {
    for status in [ClaimStatus::Confirmed, ClaimStatus::Candidate] {
        let dto = claim("table_alias", "account managers", None, status);
        let line = knowledge_learned_text(&dto);
        assert!(
            !line.contains(dto.claim_id.as_str()),
            "{status:?} line leaked the id: {line}"
        );
        assert!(
            !line.contains("aaaaaa"),
            "{status:?} line leaked an id prefix: {line}"
        );
    }
}

#[test]
fn a_column_scoped_fact_names_its_column() {
    let line = knowledge_learned_text(&claim(
        "column_role",
        "the revenue measure",
        Some("amount"),
        ClaimStatus::Confirmed,
    ));
    assert!(
        line.contains("amount as the revenue measure"),
        "names column and role: {line}"
    );
}

#[test]
fn a_long_description_is_bounded_to_one_line() {
    let long = "x".repeat(200);
    let line = knowledge_learned_text(&claim(
        "table_description",
        &long,
        None,
        ClaimStatus::Confirmed,
    ));
    assert!(line.contains('…'), "long value is elided: {line}");
    // The value is what is unbounded; the rest of the line is fixed wording and
    // an object name. Assert the value's contribution, not a total that would
    // move with every object name.
    let value_run = line.chars().filter(|c| *c == 'x').count();
    assert!(
        value_run <= 60,
        "the value is bounded before it reaches the line: {value_run} chars"
    );
}

/// Multi-byte values must not panic the elision.
#[test]
fn eliding_cuts_on_a_character_boundary() {
    let wide = "é".repeat(100);
    let line = knowledge_learned_text(&claim("table_grain", &wide, None, ClaimStatus::Confirmed));
    assert!(line.contains('…'), "wide value is elided: {line}");
}

/// A kind this renderer does not know is silence, not a raw token on screen.
#[test]
fn an_unknown_kind_renders_nothing() {
    assert!(
        knowledge_learned_text(&claim(
            "relationship",
            "orders→customers",
            None,
            ClaimStatus::Confirmed
        ))
        .is_empty(),
        "an unrendered kind is silent"
    );
}

/// A column-scoped kind arriving without its column would otherwise print a
/// dangling phrase.
#[test]
fn a_column_kind_without_a_column_renders_nothing() {
    assert!(
        knowledge_learned_text(&claim(
            "column_role",
            "a measure",
            None,
            ClaimStatus::Confirmed
        ))
        .is_empty(),
        "no column, no line"
    );
}
