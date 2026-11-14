#[tokio::test]
async fn iso_date_anchors_at_2026_01_01() {
    use super::calendar::iso_date;
    assert_eq!(iso_date(0), "2026-01-01");
    assert_eq!(
        iso_date(90),
        "2025-10-03",
        "the last 90 days start past 2025-10-03"
    );
    assert_eq!(iso_date(366), "2024-12-31");
}
