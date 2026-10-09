//! The advertised tool surface: join_check is offered with its arguments
//! only when data sharing is on.

use super::*;

#[test]
fn join_check_is_advertised_with_its_arguments_when_sharing_is_on() {
    let tools = DatabaseTools::definitions(true, false, false, false, true);
    let tool = tools
        .iter()
        .find(|tool| tool.name == "join_check")
        .expect("join_check is advertised when data sharing is on");
    assert!(tool.read_only);
    assert!(tool.effect.requires_approval);
    assert_eq!(tool.parameters["required"], serde_json::json!(["sql"]));
    assert!(tool.parameters["properties"]["connection"].is_object());
    assert!(tool.parameters["properties"]["sql"].is_object());

    let hidden = DatabaseTools::definitions(false, false, false, false, false);
    assert!(
        !hidden.iter().any(|tool| tool.name == "join_check"),
        "join_check is not advertised when data sharing is off"
    );
}
