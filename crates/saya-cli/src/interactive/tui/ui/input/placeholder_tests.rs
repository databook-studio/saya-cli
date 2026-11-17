use super::INPUT_PLACEHOLDER;

/// The empty box invites a plain message, the default way to work. `/start`
/// is documented in `/help`, not pushed at the first keystroke.
#[test]
fn the_placeholder_invites_a_plain_message_only() {
    assert!(
        !INPUT_PLACEHOLDER.contains("/start"),
        "the placeholder must not mention /start: {INPUT_PLACEHOLDER}"
    );
    assert!(
        INPUT_PLACEHOLDER.contains("question") || INPUT_PLACEHOLDER.contains("describe"),
        "the placeholder invites a message: {INPUT_PLACEHOLDER}"
    );
}
