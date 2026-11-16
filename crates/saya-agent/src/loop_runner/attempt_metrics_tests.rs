use super::{AttemptTiming, duration_ns};
use std::time::Duration;

#[test]
fn nanosecond_conversion_and_read_sum_saturate_at_u64() {
    let huge = Duration::new(u64::MAX, 1);
    assert_eq!(duration_ns(huge), u64::MAX);

    let mut timing = AttemptTiming::new();
    timing.add_read_elapsed(huge);
    timing.add_read_elapsed(Duration::from_nanos(1));
    assert_eq!(timing.read_elapsed_ns, u64::MAX);
}
