/// The saved record rides the user turn exactly once, after the
/// application-continuation block; the notes stay out of the system prompt;
/// the hostile close-sentinel in the notes stays escaped data; the file fact
/// is re-checked against disk (changed) and the check reads historical.
#[tokio::test]
async fn the_record_rides_the_user_turn_once_after_the_application_block() {
    let (result, captured) = run_record_turn(true, true).await;
    result.expect("the turn completes");
    assert_eq!(captured.request_count(), 1, "one answering request");
    let messages = captured
        .first_messages()
        .expect("the first request was recorded");
    let system = &messages[0];
    let user = messages.last().unwrap();
    assert_eq!(system.role, "system");
    assert_eq!(user.role, "user");

    assert_eq!(
        user.content.matches("source: continuation-record").count(),
        1,
        "exactly one record block: {}",
        user.content
    );
    let application_at = user
        .content
        .find("source: application-continuation")
        .expect("the application block rides the same turn");
    let record_at = user
        .content
        .find("source: continuation-record")
        .expect("the record block is present");
    assert!(
        application_at < record_at,
        "the record block follows the application block"
    );
    assert!(
        !system.content.contains("remember the migration check"),
        "notes must never reach the system prompt: {}",
        system.content
    );
    assert!(
        user.content.contains("remember the migration check"),
        "the notes reach the user turn as data"
    );
    assert!(
        user.content.contains("<<<\\CONTEXT_BLOCK_END>>>"),
        "the hostile close-sentinel is escaped data"
    );
    assert_eq!(
        user.content.matches("<<<CONTEXT_BLOCK_END>>>").count(),
        user.content.matches("<<<CONTEXT_BLOCK_BEGIN>>>").count(),
        "every structural close is accounted for; the body's sentinel is inert"
    );
    assert!(
        user.content.contains("changed since observed"),
        "the file drift was refreshed from the bound workspace: {}",
        user.content
    );
    assert!(
        user.content.contains("— passed (observed"),
        "the check outcome rides its line: {}",
        user.content
    );
    assert!(
        user.content.contains(CHECKS_HEADING),
        "the historical heading is present"
    );
}

/// Without a carried record the turn renders no record block at all.
#[tokio::test]
async fn without_a_record_the_turn_carries_no_record_block() {
    let (result, captured) = run_record_turn(false, true).await;
    result.expect("the turn completes");
    let messages = captured
        .first_messages()
        .expect("the first request was recorded");
    let user = messages.last().unwrap();
    assert_eq!(
        user.content.matches("source: continuation-record").count(),
        0,
        "no record, no block: {}",
        user.content
    );
    assert!(
        !user.content.contains("remember the migration check"),
        "no note may leak without a record"
    );
}

/// The record block is not gated by the data-sharing flag: user notes and
/// tool-observed facts are not database cells. This run flips
/// `allow_data_sharing` (the `allow_query_data` input) to false and the block
/// still rides.
#[tokio::test]
async fn the_record_block_survives_a_closed_data_lane() {
    let (result, captured) = run_record_turn(true, false).await;
    result.expect("the turn completes");
    let messages = captured
        .first_messages()
        .expect("the first request was recorded");
    let user = messages.last().unwrap();
    assert_eq!(
        user.content.matches("source: continuation-record").count(),
        1,
        "the record block is not gated by allow_query_data: {}",
        user.content
    );
}
