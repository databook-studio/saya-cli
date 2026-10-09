/// The event serializes under its `clarification_needed` type tag with the
/// question and options carried, so JSON/NDJSON consumers read it structurally.
#[test]
fn clarification_needed_serializes_under_its_type_tag() {
    let event = AgentEvent::clarification_needed(
        "Which metric should \"active users\" use?",
        vec!["sessions in the last 30 days".into()],
    );
    let json = serde_json::to_string(&event).expect("serializes");
    assert!(
        json.contains(r#""type":"clarification_needed""#),
        "type tag: {json}"
    );
    assert!(json.contains("active users"), "question carried: {json}");
    assert!(
        json.contains("sessions in the last 30 days"),
        "options carried: {json}"
    );
}

/// The argument bounds: exactly the shape `request_clarification` accepts —
/// a non-empty question up to 300 characters, and up to six options of up to
/// 80 characters each. Over any bound the call is refused with a named reason.
#[test]
fn the_argument_bounds_are_enforced_with_named_reasons() {
    let ok = parse(&serde_json::json!({
        "question": "Which metric should \"active users\" use?",
        "options": ["a", "b", "c", "d", "e", "f"]
    }))
    .expect("six options are the bound, not past it");
    assert_eq!(ok.options.len(), 6);
    assert!(parse(&serde_json::json!({"question": "which one"})).is_ok());

    let over = parse(&serde_json::json!({"question": "x".repeat(301)}))
        .expect_err("a question over the bound is refused");
    assert!(over.to_string().contains("300"), "{over}");
    let empty =
        parse(&serde_json::json!({"question": "   "})).expect_err("an empty question is refused");
    assert!(empty.to_string().contains("empty"), "{empty}");
    let not_string =
        parse(&serde_json::json!({"question": 7})).expect_err("a non-string question is refused");
    assert!(not_string.to_string().contains("string"), "{not_string}");
    let many = parse(&serde_json::json!({
        "question": "which one",
        "options": ["a", "b", "c", "d", "e", "f", "g"]
    }))
    .expect_err("a seventh option is refused");
    assert!(many.to_string().contains("6"), "{many}");
    let long_option = parse(&serde_json::json!({
        "question": "which one",
        "options": ["x".repeat(81)]
    }))
    .expect_err("an option over the bound is refused");
    assert!(long_option.to_string().contains("80"), "{long_option}");
    let not_array = parse(&serde_json::json!({
        "question": "which one",
        "options": "a"
    }))
    .expect_err("non-array options are refused");
    assert!(not_array.to_string().contains("array"), "{not_array}");
    let not_object = parse(&serde_json::json!("question")).expect_err("non-object args refused");
    assert!(not_object.to_string().contains("object"), "{not_object}");
}

/// Sanitisation: control characters never ride the event — the question and
/// each option are cleaned before the loop emits anything.
#[test]
fn the_question_and_options_are_sanitised_and_trimmed() {
    let parsed = parse(&serde_json::json!({
        "question": "\u{001b}[31mwhich metric\u{0007} is right?  ",
        "options": ["  \u{0002}sessions in 30 days  ", "purchases\u{007f}"]
    }))
    .expect("sanitised input still parses");
    assert_eq!(parsed.question, "[31mwhich metric is right?");
    assert_eq!(parsed.options[0], "sessions in 30 days");
    assert_eq!(parsed.options[1], "purchases");
}
