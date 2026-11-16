//! The grammar's shape rule: prefix-shaped grants refuse at parse.

/// Prefix-shaped grants refuse at parse.
#[test]
fn prefix_shaped_grants_refuse_at_parse() {
    let error = match crate::commands::run::scopes::parse(
        &["command:cargo test".to_owned()],
        crate::commands::run::scopes::Surface::Session,
    ) {
        Err(error) => error,
        Ok(approved) => panic!(
            "a prefix grant is a comforting fiction and must refuse: {:?}",
            approved.tokens
        ),
    };
    assert!(
        error.contains("bare name"),
        "the refusal states the shape rule: {error}"
    );
}
