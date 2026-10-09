use super::*;
use crate::runner::error::RunnerError;
use saya_types::RunnerScope;

#[test]
fn argv_count_item_and_total_bounds_refuse_before_spawn() {
    let allowed = RunnerScope::new(vec!["tool".to_owned()]).unwrap();
    let too_many = vec![String::new(); MAX_ARG_COUNT + 1];
    assert!(matches!(
        validate_call(
            &allowed,
            Path::new("/does/not/exist"),
            Duration::from_secs(5),
            None,
            "tool",
            &too_many,
        ),
        Err(RunnerError::ArgsNotTyped { .. })
    ));

    let too_long = vec!["x".repeat(MAX_ARG_BYTES + 1)];
    assert!(matches!(
        validate_call(
            &allowed,
            Path::new("/does/not/exist"),
            Duration::from_secs(5),
            None,
            "tool",
            &too_long,
        ),
        Err(RunnerError::ArgsNotTyped { .. })
    ));

    let item = "x".repeat(MAX_ARG_BYTES);
    let too_wide = vec![item; MAX_ARGV_BYTES / MAX_ARG_BYTES + 1];
    assert!(matches!(
        validate_call(
            &allowed,
            Path::new("/does/not/exist"),
            Duration::from_secs(5),
            None,
            "tool",
            &too_wide,
        ),
        Err(RunnerError::ArgsNotTyped { .. })
    ));
}
