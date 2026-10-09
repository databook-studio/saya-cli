use super::*;

#[test]
fn result_shape_is_advertised_with_its_arguments_when_sharing_is_on() {
    let tools = DatabaseTools::definitions(true, false, false, false, true);
    let tool = tools
        .iter()
        .find(|tool| tool.name == "result_shape")
        .expect("result_shape is advertised when data sharing is on");
    assert!(tool.read_only);
    assert!(tool.effect.requires_approval);
    assert_eq!(tool.parameters["required"], serde_json::json!(["sql"]));
    assert!(tool.parameters["properties"]["connection"].is_object());
    assert!(tool.parameters["properties"]["sql"].is_object());

    let hidden = DatabaseTools::definitions(false, false, false, false, false);
    assert!(
        !hidden.iter().any(|tool| tool.name == "result_shape"),
        "result_shape is not advertised when data sharing is off"
    );
}
