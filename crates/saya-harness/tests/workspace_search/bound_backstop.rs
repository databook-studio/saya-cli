//! The direct-call backstop: even a caller holding the surface itself cannot
//! widen any declared bound past the workspace constants.

use saya_harness::{
    HarnessError,
    workspace::{MAX_IO_BYTES, MAX_LIST_ENTRIES},
};

use super::Sandbox;

#[test]
fn direct_search_call_cannot_widen_any_bound() {
    let sandbox = Sandbox::new("search-cap-backstop");
    assert!(matches!(
        sandbox.ws.glob("**", MAX_LIST_ENTRIES + 1, 1),
        Err(HarnessError::BoundsExceeded { .. })
    ));
    assert!(matches!(
        sandbox
            .ws
            .grep("needle", false, 1, 1, (MAX_IO_BYTES + 1) as u64, 1),
        Err(HarnessError::BoundsExceeded { .. })
    ));
}
