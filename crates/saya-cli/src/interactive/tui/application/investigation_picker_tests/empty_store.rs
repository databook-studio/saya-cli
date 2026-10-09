//! The empty-store state: opening the picker with zero saved investigations
//! never opens an overlay, and the remedy is said instead.

use crate::interactive::tui::application::tests_support::idle_app;
use std::time::Duration;

/// Opening with zero saved investigations never opens an overlay: the
/// remedy is said instead.
#[test]
fn opening_with_no_saved_investigations_says_the_remedy() {
    let mut app = idle_app();
    app.open_investigation_picker();
    for _ in 0..200 {
        app.poll_investigation_picker();
        if app.overlays.investigations_loading.is_none() {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        app.overlays.investigations_loading.is_none(),
        "the load completed"
    );
    assert!(
        app.overlays.investigations.is_none(),
        "no overlay for an empty store"
    );
    let block = app
        .transcript
        .blocks()
        .last()
        .expect("the empty state is said");
    assert!(
        block
            .text
            .contains("No saved investigations — save one with /investigation save <name>"),
        "the remedy is named: {:?}",
        block.text
    );
}
