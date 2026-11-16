use super::*;

#[test]
fn claim_payload_version_is_three() {
    // claim-reasons adds an optional `reason` to the directive variants.
    // The field is `#[serde(default)]`, so a version-2 row decodes with
    // `reason: None`; no migration is owed. A claim proposed before this
    // change carries version 2 (or 1) and must still decode.
    assert_eq!(CLAIM_PAYLOAD_VERSION, 3);
}
