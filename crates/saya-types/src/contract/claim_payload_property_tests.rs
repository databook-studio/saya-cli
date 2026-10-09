//! Property 5 (spec §1): every `ClaimPayload` constructor either returns a
//! payload whose text round-trips the input, or a typed error — never a
//! panic, never a payload containing a control character. Pure. This is the
//! generalisation of the hand-written constructor tests: the constructors are
//! the only thing keeping oversized and control-character-bearing text out of
//! a rendered context block, and a property asks whether that holds across an
//! input space the hand-picked cases did not imagine.
use super::*;
use crate::contract::claim_enums::{Cardinality, ColumnRole};
use crate::{DatabaseObjectKind, ProfileIdentity};
use proptest::prelude::*;

fn make_profile() -> ProfileIdentity {
    ProfileIdentity::parse(&format!("p-{}", "a".repeat(64))).unwrap()
}

fn make_target() -> DatabaseObjectRef {
    let profile = make_profile();
    DatabaseObjectRef::new(profile, "c", "s", "t", DatabaseObjectKind::Table).unwrap()
}

/// Any text a user might supply as a description/alias/grain/value: empty,
/// long, control chars, unicode — every rejection path must fire, none panic.
fn text() -> impl Strategy<Value = String> {
    prop::collection::vec(any::<char>(), 0..=1100).prop_map(|chars| chars.into_iter().collect())
}

/// Any column name: same wide alphabet, shorter cap (`MAX_NAME_CHARS`).
fn col_name() -> impl Strategy<Value = String> {
    prop::collection::vec(any::<char>(), 0..=130).prop_map(|chars| chars.into_iter().collect())
}

/// True when `s` contains any Unicode control character.
fn has_control(s: &str) -> bool {
    s.chars().any(|c| c.is_control())
}

/// All five column roles. `select` needs a `Vec`; an array does not satisfy
/// `Into<Cow<'static, [_]>>`.
fn role() -> impl Strategy<Value = ColumnRole> {
    prop::sample::select(vec![
        ColumnRole::Identifier,
        ColumnRole::Dimension,
        ColumnRole::Measure,
        ColumnRole::Timestamp,
        ColumnRole::Sensitive,
    ])
}

/// Every cardinality.
fn cardinality() -> impl Strategy<Value = Cardinality> {
    prop::sample::select(vec![
        Cardinality::OneToOne,
        Cardinality::OneToMany,
        Cardinality::ManyToOne,
        Cardinality::ManyToMany,
    ])
}

#[path = "claim_payload_property_tests/column_fields.rs"]
mod column_fields;
#[path = "claim_payload_property_tests/join_rules.rs"]
mod join_rules;
#[path = "claim_payload_property_tests/metric_definitions.rs"]
mod metric_definitions;
#[path = "claim_payload_property_tests/name_fields.rs"]
mod name_fields;
#[path = "claim_payload_property_tests/relationships.rs"]
mod relationships;
#[path = "claim_payload_property_tests/text_fields.rs"]
mod text_fields;
