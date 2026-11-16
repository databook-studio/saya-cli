//! Additive terminal failure records retain strict identities and closed categories.
use saya_types::{CodingEventKind, CodingEventV1, JobState};
use serde_json::json;
#[test]
fn terminal_failure_wire_rejects_malformed_identity_state_and_private_text() {
    let valid = json!({"version":1,"sequence":2,"job_id":"job-a","event":{"type":"run_terminated","state":"failed","failure":{"class":"provider_error","agent_id":"child-a","invocation_id":"invocation-a"},"secondary":"diagnostic_storage","diagnostic_fault":"unsafe_path"}});
    let decoded = CodingEventV1::from_json_bytes(&serde_json::to_vec(&valid).unwrap()).unwrap();
    assert!(matches!(
        decoded.event(),
        CodingEventKind::RunTerminated {
            state: JobState::Failed,
            ..
        }
    ));
    for invalid in [
        {
            let mut value = valid.clone();
            value["event"]["state"] = json!("completed");
            value
        },
        {
            let mut value = valid.clone();
            value["event"]["failure"]["invocation_id"] = json!(null);
            value
        },
        {
            let mut value = valid.clone();
            value["agent_id"] = json!("child-a");
            value["invocation_id"] = json!("invocation-a");
            value
        },
        {
            let mut value = valid.clone();
            value["event"]["failure"]["class"] = json!("PRIVATE_ERROR_SENTINEL");
            value
        },
        {
            let mut value = valid.clone();
            value["event"]["failure"]["detail"] = json!("PRIVATE_ERROR_SENTINEL");
            value
        },
        {
            let mut value = valid.clone();
            value["event"]["diagnostic_fault"] = json!("PRIVATE_ERROR_SENTINEL");
            value
        },
    ] {
        assert!(
            CodingEventV1::from_json_bytes(&serde_json::to_vec(&invalid).unwrap()).is_err(),
            "accepted malformed terminal record: {invalid}"
        );
    }
}
#[test]
fn old_state_event_remains_readable_without_failure_fields() {
    let old = br#"{"version":1,"sequence":1,"job_id":"job-a","event":{"type":"state_changed","state":"failed"}}"#;
    assert!(matches!(
        CodingEventV1::from_json_bytes(old).unwrap().event(),
        CodingEventKind::StateChanged {
            state: JobState::Failed
        }
    ));
}
