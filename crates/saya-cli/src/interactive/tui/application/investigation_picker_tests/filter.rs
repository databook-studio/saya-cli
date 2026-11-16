//! The pure filter over saved investigations: a case-insensitive substring
//! match across the id and the display name, keeping every entry for an
//! empty query.

use super::super::filter_entries;
use super::three_entries;

/// The filter is a case-insensitive substring match over the id and the
/// display name; the empty query keeps every entry.
#[test]
fn filter_matches_id_and_name_case_insensitively() {
    let entries = three_entries();
    let visible = filter_entries(&entries, "re");
    assert_eq!(visible.len(), 2, "recent-orders and cost-by-region match");
    assert_eq!(filter_entries(&entries, "RECENT").len(), 1);
    assert_eq!(
        filter_entries(&entries, "REGION").first().unwrap().id,
        "cost-by-region-deadbeef"
    );
    assert_eq!(filter_entries(&entries, "").len(), 3);
    assert!(filter_entries(&entries, "zzz").is_empty());
}
