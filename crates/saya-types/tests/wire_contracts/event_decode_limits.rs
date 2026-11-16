use saya_types::{CodingEventError, CodingEventKind, CodingEventV1, JobId, JobState};

const VALID_EVENT: &[u8] =
    br#"{"version":1,"sequence":1,"job_id":"job-1","event":{"type":"state_changed","state":"queued"}}"#;

#[test]
fn decoder_accepts_exact_limit_and_reports_first_oversize_length() {
    let mut exact_limit = VALID_EVENT.to_vec();
    exact_limit.resize(saya_types::MAX_CODING_EVENT_BYTES, b' ');
    let decoded = CodingEventV1::from_json_bytes(&exact_limit).unwrap();
    assert_eq!(*decoded.job_id(), JobId::parse("job-1").unwrap());
    assert!(matches!(
        decoded.event(),
        CodingEventKind::StateChanged {
            state: JobState::Queued
        }
    ));

    let mut over_limit = exact_limit;
    over_limit.push(b' ');
    assert_eq!(
        CodingEventV1::from_json_bytes(&over_limit),
        Err(CodingEventError::Oversize(
            saya_types::MAX_CODING_EVENT_BYTES + 1
        ))
    );
}
