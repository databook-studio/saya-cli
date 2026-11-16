#[test]
fn elapsed_deadline_cancels_pending_terminal_approval_without_a_second_request() {
    warm_up_saya();
    // The budget must outlast startup + the first provider round trip on a loaded machine; the test still ends with the deadline cancelling the pending approval.
    run_pending_approval_cancellation("deadline", 4, None);
}

#[test]
fn bare_esc_cancels_pending_terminal_approval_before_a_long_deadline() {
    warm_up_saya();
    run_pending_approval_cancellation("esc", 60, Some(b"\x1b"));
}

#[test]
fn modified_keys_do_not_approve_and_plain_a_approves_the_real_sql_result() {
    warm_up_saya();
    run_approval_key("ctrl-a", b"\x01\r", false);
    run_approval_key("alt-a", b"\x1ba\r", false);
    run_approval_key("plain-a", b"a\r", true);
}
