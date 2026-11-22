#[path = "session_capacity.rs"]
pub(crate) mod capacity;
#[cfg(test)]
#[path = "capacity_test_support.rs"]
pub(crate) mod capacity_test_support;
#[path = "notes.rs"]
pub(crate) mod notes;
#[path = "session_paths.rs"]
pub(crate) mod paths;
#[path = "session_recovery.rs"]
pub(crate) mod recovery;
#[path = "session_resume.rs"]
// The old surface was this seam's only caller; X1 deleted it. The code here
// is X2/X3's to remove, kept compiled rather than deleted twice.
#[allow(dead_code)]
pub(crate) mod resume;
#[path = "session_rotate.rs"]
pub(crate) mod rotate;
#[path = "session_rotate_run.rs"]
pub(crate) mod rotate_run;

use rotate as session_rotate;
