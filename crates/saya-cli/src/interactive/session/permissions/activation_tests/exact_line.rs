use super::super::{BYPASS_ON, HOST_LANE_FACT, NO_INTERPRETERS_STAGED, bypass_line};

/// The activation line's current bytes, whole: the mode fact, the
/// none-staged sentence, and the uncomposed line's end. Nothing stages
/// interpreters (`[jobs.interpreter]` is a parse error), so this is the only
/// plain line the product can emit.
#[test]
fn the_plain_activation_line_keeps_its_bytes() {
    assert_eq!(
        bypass_line(false, &[]),
        format!("{BYPASS_ON}\n{NO_INTERPRETERS_STAGED}"),
        "the plain line is the mode fact and the none-staged sentence, nothing else"
    );
}

/// The composed line's current bytes, whole: the lane fact and the
/// denied-names segment each start their own line after the two base facts.
#[test]
fn the_composed_activation_line_keeps_its_bytes() {
    let denied = vec!["curl".to_string(), "ssh".to_string()];
    assert_eq!(
        bypass_line(true, &denied),
        format!(
            "{BYPASS_ON}\n{NO_INTERPRETERS_STAGED}\n{HOST_LANE_FACT}\n\
             denied for this session: curl, ssh"
        ),
        "the composed line appends the lane fact then the denied names"
    );
}
