/// A seeded token and a suggested token are the same string for the same
/// destination: `/allow` parses through the grammar's parser, which
/// normalises the fetch family to the URL parser's spelling — the spelling
/// the suggester produces from the call's URL. Whatever casing the user
/// types, the grant pre-answers the call it names: the seed and the
/// suggestion are normalised to one spelling, so `/grants` lists a grant that
/// pre-answers its call.
#[test]
fn a_seeded_token_and_a_suggested_token_are_the_same_string_for_a_destination() {
    let suggested = token_for(
        "http_fetch",
        json!({"url": "https://Example.com/x"}),
        &composed_facts(),
    );
    let approved = scopes::parse(
        &["fetch:HTTPS+Example.com".to_string()],
        scopes::Surface::Session,
    )
    .expect("a mixed-case fetch token parses on the session surface");
    assert_eq!(
        approved.tokens,
        vec![suggested],
        "the seeded token and the suggested token are one string for one destination"
    );
}

/// A malformed or absent argument yields `None`, never a guessed token —
/// and `None` means the tool keeps asking every call. Judged against the
/// fully composed facts: a malformed argument yields nothing even where the
/// composition could have carried the destination.
#[test]
fn a_malformed_or_absent_argument_yields_none_never_a_token() {
    let facts = composed_facts();
    assert_eq!(
        grant_token("http_fetch", &json!({}), None, &facts),
        None,
        "no url"
    );
    assert_eq!(
        grant_token("http_fetch", &json!({"url": "not a url"}), None, &facts),
        None,
        "not a URL"
    );
    assert_eq!(
        grant_token(
            "http_fetch",
            &json!({"url": "mailto:someone@example.com"}),
            None,
            &facts
        ),
        None,
        "a scheme with no host"
    );
    assert_eq!(
        grant_token("http_fetch", &json!({"url": ""}), None, &facts),
        None,
        "empty url"
    );
    assert_eq!(
        grant_token("http_fetch", &json!({"url": 7}), None, &facts),
        None,
        "non-string url"
    );
    assert_eq!(
        grant_token(
            "http_download",
            &json!({"destination": "f.bin"}),
            None,
            &facts
        ),
        None,
        "download without url"
    );
}
