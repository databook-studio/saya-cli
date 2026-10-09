use super::super::bypass_line;

/// Where the probe refused, the activation line carries the same fact the
/// notice seam says: "everything runs" with an absent runner is a claim the
/// session must qualify, at the moment it says bypass on.
#[test]
fn the_activation_line_references_the_probe_refusal() {
    let line = bypass_line(&["python3".to_string()], true, false, &[]);
    assert!(
        line.contains("run_program is unavailable: the sandbox probe did not prove this host"),
        "the probe-refused fact is carried on the activation line: {line}"
    );
    let proven = bypass_line(&["python3".to_string()], false, false, &[]);
    assert!(
        !proven.contains("run_program is unavailable"),
        "a proven host says nothing about the probe: {proven}"
    );
}
