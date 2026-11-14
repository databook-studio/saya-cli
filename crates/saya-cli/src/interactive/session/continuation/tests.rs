use super::truncate_utf8;

#[test]
fn question_truncation_stops_at_a_utf8_boundary() {
    assert_eq!(truncate_utf8("abé🦀z", 5), "abé");
    assert_eq!(truncate_utf8("é", 1), "");
}
