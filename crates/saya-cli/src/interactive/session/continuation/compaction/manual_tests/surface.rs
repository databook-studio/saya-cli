/// `/compact` is registered, listed, described, and helped — the parity the
/// `/mode` slice established across the four hand-maintained touchpoints.
#[test]
fn compact_is_registered_listed_described_and_helped() {
    assert_eq!(
        crate::slash::parse_slash_command("/compact"),
        Ok(Some(crate::slash::SlashCommand::Compact))
    );
    assert!(
        crate::slash::registry::KNOWN_COMMANDS.contains(&"compact"),
        "compact is registered"
    );
    assert!(
        crate::slash::description_for("compact").is_some(),
        "compact has a popup description"
    );
    let listing = crate::slash::help_for(None);
    assert!(
        listing.contains("/compact"),
        "the listing shows /compact: {listing}"
    );
    let help = crate::slash::help_for(Some("compact"));
    assert!(
        help.contains("transcript is unchanged"),
        "the help states the transcript guarantee: {help}"
    );
    assert!(
        help.contains("Automatic at 95%"),
        "the help states the automatic trigger and its threshold: {help}"
    );
    assert!(
        help.contains("`manual`") && help.contains("`off`"),
        "the help states the two non-automatic modes: {help}"
    );
}

/// `/clear` resets a compaction along with the conversation, so a fresh
/// context replays no stale summary.
#[test]
fn clear_resets_a_compaction() {
    let mut state = turns(5);
    let history = state.provider_history();
    let plan = plan(&state.turns, &history).expect("long history plans");
    apply(&mut state, &plan, "older turns.").expect("applies");
    assert!(state.compaction_summary.is_some());
    state.apply(crate::slash::SlashCommand::Clear, &[]);
    assert!(
        state.compaction_summary.is_none() && state.compacted_turns == 0,
        "/clear must reset the compaction with the conversation"
    );
}

/// The success message keeps its shape: the count, the estimate, and the
/// transcript guarantee with its remedy.
#[test]
fn the_success_message_keeps_its_shape() {
    let message = success_message(3, "older turns discussed orders at length");
    assert!(
        message.starts_with("Compacted 3 turns into a summary (~6 tokens)."),
        "the count and estimate lead: {message}"
    );
    assert!(
        message.contains("The transcript is unchanged; /export first if you want the full text."),
        "the guarantee and remedy follow: {message}"
    );
}

/// Pinned-token extraction pins SQL statements: the tool-call arguments are
/// the only record of what ran, so the summary must carry them.
#[test]
fn sql_statements_are_pinned() {
    let summarise = vec![
        ChatMessage::text("user", "count customers"),
        ChatMessage::text("assistant", "SELECT customer, age FROM users gave 42 rows"),
    ];
    let pinned = pinned_tokens(&summarise);
    assert!(
        pinned
            .iter()
            .any(|token| token.contains("SELECT customer, age FROM users")),
        "the statement is pinned: {pinned:?}"
    );
}
