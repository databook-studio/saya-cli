/// `/grants` on an empty store: the lifetime header with the zero count and
/// an explicit empty state — never a bare nothing.
#[test]
fn the_listing_states_the_empty_store_explicitly() {
    assert_eq!(
        listing(ApprovalPolicy::Ask, &SessionGrants::default()),
        "session grants (die with this session): 0\n  (none — nothing pre-answers \
         this session yet; /allow <scopes> or answer [s] at an ask)"
    );
}

/// `/grants` under bypass states the mode **first**: the count alone would
/// read "nothing runs", when the truth is everything does — every call runs
/// without asking and the store is not consulted. The listing follows.
#[test]
fn the_listing_states_the_mode_first_under_bypass() {
    let grants = store_with("runner:bench");
    let text = listing(ApprovalPolicy::Bypass, &grants);
    let (mode_line, rest) = text
        .split_once('\n')
        .expect("the bypass listing carries the mode line first");
    assert_eq!(
        mode_line, "mode bypass: every call runs without asking; grants are not consulted",
        "the mode is stated before the listing: {text}"
    );
    assert!(
        rest.contains("session grants (die with this session): 1") && rest.contains("runner:bench"),
        "the listing follows the mode line: {rest}"
    );
    // The mode line comes before the count, byte-ordered.
    let count = text
        .find("session grants (die with this session): 1")
        .expect("the count line is present");
    let mode = text.find("mode bypass:").expect("the mode line is present");
    assert!(mode < count, "the mode line precedes the listing: {text}");
}

/// The non-bypass modes render today's listing exactly: no mode line, the
/// lifetime header, the count, and the tokens.
#[test]
fn the_listing_without_bypass_keeps_its_exact_bytes() {
    let grants = store_with("sql:analytics");
    grants.grant("runner:bench");
    for mode in [
        ApprovalPolicy::Ask,
        ApprovalPolicy::ReadOnly,
        ApprovalPolicy::Never,
    ] {
        assert_eq!(
            listing(mode, &grants),
            "session grants (die with this session): 2\n  runner:bench\n  sql:analytics",
            "no mode line under {mode:?} — the listing is today's bytes"
        );
    }
}

/// `/grants` lists the store's tokens **verbatim**, one per line, sorted,
/// under a header stating the lifetime, with a count: the words are the
/// record, and they are the same words the prompt offered.
#[test]
fn the_listing_prints_the_tokens_verbatim_sorted() {
    let grants = store_with("sql:analytics");
    grants.grant("runner:bench");
    assert_eq!(
        listing(ApprovalPolicy::Ask, &grants),
        "session grants (die with this session): 2\n  runner:bench\n  sql:analytics",
        "sorted, one per line, under the lifetime header with the count"
    );
}
