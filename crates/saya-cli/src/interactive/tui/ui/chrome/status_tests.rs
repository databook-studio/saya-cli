//! Status-bar tests: the approval colour map pins the mode grammar.
//!
//! Approval no longer paints on the bottom bar — it removed the posture
//! entirely (see `status_split_tests.rs`), leaving it to the top context
//! line alone. `approval_colour` stays as the grammar's colour mapping
//! (a caller may still reach for it), so it is pinned here directly rather
//! than through the bar it no longer paints on.

use super::super::theme::{danger, secondary, success, warning};
use super::approval_colour;

/// The colour map carries an explicit arm for every mode the grammar
/// parses; the catch-all (`secondary()`) is the quiet-drift hole a fourth
/// variant would fall into. `bypass` renders `danger()` red: the mode
/// that claims "everything runs" must read as the danger it is, in the
/// grammar's own word, on every surface.
#[test]
fn the_status_colour_map_has_an_arm_for_every_mode() {
    for (mode, expected) in [
        ("read-only", success()),
        ("ask", warning()),
        ("never", danger()),
        ("bypass", danger()),
    ] {
        assert_eq!(
            approval_colour(mode),
            expected,
            "{mode} must have its own colour arm"
        );
    }
    assert_eq!(
        approval_colour("whatever-a-future-parse-site-forgot"),
        secondary(),
        "the catch-all is named, not removed: unknown words stay grey"
    );
}
