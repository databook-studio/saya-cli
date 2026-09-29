//! Tests for the run-time `--param` bindings and the evidence digest: values
//! parse against the declared types, refusals name parameters never values,
//! and the digest is deterministic over the canonical declaration-ordered
//! list.

use super::{bind_values, evidence_fields};
use crate::commands::investigation::params::parse_specs;
use saya_types::{BoundParam, ParamType, ParamValue};

fn bound(name: &str, value: ParamValue) -> BoundParam {
    BoundParam {
        name: name.to_owned(),
        value,
    }
}

// --- bind_values ------------------------------------------------------------

#[test]
fn bindings_parse_in_declaration_order_and_fill_missing_optionals_with_typed_nulls() {
    let specs =
        parse_specs(&["label:string:required".to_owned(), "since:date".to_owned()]).unwrap();
    let binds = bind_values(&specs, &["label=first".to_owned()]).unwrap();
    assert_eq!(
        binds,
        vec![
            bound("label", ParamValue::String("first".to_owned())),
            bound("since", ParamValue::Null(ParamType::Date)),
        ]
    );
}

#[test]
fn bindings_parse_the_null_literal_as_a_typed_null() {
    let specs = parse_specs(&["v:integer".to_owned()]).unwrap();
    let binds = bind_values(&specs, &["v=null".to_owned()]).unwrap();
    assert_eq!(
        binds,
        vec![bound("v", ParamValue::Null(ParamType::Integer))]
    );
}

#[test]
fn bindings_parse_each_declared_type() {
    let specs = parse_specs(&[
        "s:string".to_owned(),
        "i:integer".to_owned(),
        "b:boolean".to_owned(),
        "d:decimal".to_owned(),
        "dt:date".to_owned(),
        "ts:timestamp".to_owned(),
    ])
    .unwrap();
    let binds = bind_values(
        &specs,
        &[
            "s=' OR 1=1 --".to_owned(),
            "i=-7".to_owned(),
            "b=true".to_owned(),
            "d=-0.25".to_owned(),
            "dt=2024-02-29".to_owned(),
            "ts=2024-02-29T00:00:00+05:30".to_owned(),
        ],
    )
    .unwrap();
    assert_eq!(
        binds,
        vec![
            bound("s", ParamValue::String("' OR 1=1 --".to_owned())),
            bound("i", ParamValue::Integer(-7)),
            bound("b", ParamValue::Boolean(true)),
            bound("d", ParamValue::Decimal("-0.25".to_owned())),
            bound("dt", ParamValue::Date("2024-02-29".to_owned())),
            bound(
                "ts",
                ParamValue::Timestamp("2024-02-29T00:00:00+05:30".to_owned())
            ),
        ]
    );
}

#[test]
fn bindings_refuse_unknown_duplicate_and_malformed_params() {
    let specs = parse_specs(&["label:string:required".to_owned()]).unwrap();

    let (code, message) = bind_values(&specs, &["region=east".to_owned()]).unwrap_err();
    assert_eq!(code, 2, "{message}");
    assert!(
        message.contains("region") && message.contains("label"),
        "the refusal names the unknown and the declared parameters: {message}"
    );
    assert!(
        !message.contains("east"),
        "a value must not leak: {message}"
    );

    let (code, message) = bind_values(
        &specs,
        &["region=east".to_owned(), "region=west".to_owned()],
    )
    .unwrap_err();
    assert_eq!(code, 2, "{message}");

    let (code, message) = bind_values(&specs, &["label".to_owned()]).unwrap_err();
    assert_eq!(code, 2, "{message}");
    assert!(message.contains("name=value"), "{message}");
}

#[test]
fn bindings_refuse_missing_required_with_names_and_types() {
    let specs = parse_specs(&[
        "label:string:required".to_owned(),
        "since:date:required".to_owned(),
    ])
    .unwrap();
    let (code, message) = bind_values(&specs, &[]).unwrap_err();
    assert_eq!(code, 2);
    assert!(
        message.contains("missing required parameter")
            && message.contains("label (string)")
            && message.contains("since (date)"),
        "the refusal lists each required parameter with its type: {message}"
    );
    assert!(message.contains("--param"), "{message}");
}

#[test]
fn bindings_refuse_a_value_outside_its_declared_type_without_echoing_it() {
    let specs = parse_specs(&["since:date".to_owned()]).unwrap();
    let (code, message) = bind_values(&specs, &["since=2024-13-01".to_owned()]).unwrap_err();
    assert_eq!(code, 2);
    assert!(
        message.contains("since") && message.contains("date"),
        "{message}"
    );
    assert!(
        !message.contains("2024-13-01"),
        "a value must not leak: {message}"
    );
}

#[test]
fn empty_bindings_stay_empty_without_specs() {
    assert!(bind_values(&[], &[]).unwrap().is_empty());
}

// --- evidence_fields --------------------------------------------------------

#[test]
fn evidence_fields_carry_names_and_a_digest_never_values() {
    let specs = parse_specs(&["label:string".to_owned(), "since:date".to_owned()]).unwrap();
    let binds = bind_values(&specs, &["label=first".to_owned()]).unwrap();
    let (names, digest) = evidence_fields(&binds);
    assert_eq!(names, vec!["label".to_owned(), "since".to_owned()]);
    let Some(sha) = &digest else {
        panic!("bound parameters carry a digest");
    };
    assert_eq!(sha.len(), 64, "{sha}");
    assert!(
        !sha.chars()
            .any(|c| !c.is_ascii_hexdigit() || c.is_ascii_uppercase()),
        "the digest is lowercase hex: {sha}"
    );
    assert!(
        !format!("{names:?}{digest:?}").contains("first"),
        "the evidence fields must not carry the value: {names:?}{digest:?}"
    );

    let (_, empty) = evidence_fields(&[]);
    assert_eq!(empty, None, "no parameters — no digest field");
}

#[test]
fn evidence_digest_is_deterministic_over_the_canonical_list() {
    let specs = parse_specs(&["label:string".to_owned(), "since:date".to_owned()]).unwrap();
    let first = bind_values(
        &specs,
        &["label=first".to_owned(), "since=2024-02-29".to_owned()],
    )
    .unwrap();
    let second = bind_values(
        &specs,
        &["since=2024-02-29".to_owned(), "label=first".to_owned()],
    )
    .unwrap();
    // Declaration order is canonical: the same values bind to the same
    // digest whichever order the flags arrived in.
    assert_eq!(evidence_fields(&first), evidence_fields(&second));
}
