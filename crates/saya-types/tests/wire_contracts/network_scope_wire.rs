use saya_types::ApprovalRequestV1;
use serde_json::Value;

const DNS_INPUT: &str = r#"{"version":1,"job_id":"job-7","agent_id":"agent-2","invocation_id":"inv-4","request_sequence":19,"policy_sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","revocation_epoch":0,"requested_at_unix_ms":0,"expires_at_unix_ms":5000,"scope":{"operation":"network_access","destination":{"scheme":"https","host":"DB.Example.Test"}}}"#;
const DNS_CANONICAL: &str = r#"{"version":1,"job_id":"job-7","agent_id":"agent-2","invocation_id":"inv-4","request_sequence":19,"policy_sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","revocation_epoch":0,"requested_at_unix_ms":0,"expires_at_unix_ms":5000,"scope":{"operation":"network_access","destination":{"scheme":"https","host":"db.example.test"}}}"#;
const IPV4: &str = r#"{"version":1,"job_id":"job-7","agent_id":"agent-2","invocation_id":"inv-4","request_sequence":19,"policy_sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","revocation_epoch":0,"requested_at_unix_ms":0,"expires_at_unix_ms":5000,"scope":{"operation":"network_access","destination":{"scheme":"https","host":"192.0.2.1"}}}"#;
const IPV6_EXPANDED: &str = r#"{"version":1,"job_id":"job-7","agent_id":"agent-2","invocation_id":"inv-4","request_sequence":19,"policy_sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","revocation_epoch":0,"requested_at_unix_ms":0,"expires_at_unix_ms":5000,"scope":{"operation":"network_access","destination":{"scheme":"https","host":"2001:0db8:0000:0000:0000:ff00:0042:8329"}}}"#;
const IPV6_CANONICAL: &str = r#"{"version":1,"job_id":"job-7","agent_id":"agent-2","invocation_id":"inv-4","request_sequence":19,"policy_sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","revocation_epoch":0,"requested_at_unix_ms":0,"expires_at_unix_ms":5000,"scope":{"operation":"network_access","destination":{"scheme":"https","host":"2001:db8::ff00:42:8329"}}}"#;

fn assert_wire_canonicalized(input: &str, expected: &str) {
    let decoded = ApprovalRequestV1::from_json_bytes(input.as_bytes()).unwrap();
    let actual: Value = serde_json::from_slice(&decoded.to_json_bytes().unwrap()).unwrap();
    assert_eq!(actual, serde_json::from_str::<Value>(expected).unwrap());

    let expected_decoded = ApprovalRequestV1::from_json_bytes(expected.as_bytes()).unwrap();
    assert_eq!(
        expected_decoded.to_json_bytes().unwrap(),
        decoded.to_json_bytes().unwrap()
    );
}

#[test]
fn network_request_wire_canonicalizes_fixed_dns_ipv4_and_ipv6_fixtures() {
    assert_wire_canonicalized(DNS_INPUT, DNS_CANONICAL);
    assert_wire_canonicalized(IPV4, IPV4);
    assert_wire_canonicalized(IPV6_EXPANDED, IPV6_CANONICAL);
}
