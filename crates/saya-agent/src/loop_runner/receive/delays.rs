use super::super::attempt::RETRY_LIMIT;
use crate::providers::MAX_RETRY_DELAY;
use std::time::Duration;

/// Fits a provider's configured backoff to the loop's retry ceiling: one retry
/// per delay, in order, and never more than `RETRY_LIMIT`. An empty schedule
/// means no retry at all. Each delay is held to the same ceiling the provider
/// settings enforce, so a provider that was not built from validated settings
/// cannot stall the loop.
pub(super) fn turn_retry_delays(configured: &[Duration]) -> Vec<Duration> {
    configured
        .iter()
        .take(usize::from(RETRY_LIMIT))
        .map(|delay| (*delay).min(MAX_RETRY_DELAY))
        .collect()
}

/// The attempts a schedule allows: the first plus one per retry delay.
pub(super) fn max_attempts(delays: &[Duration]) -> u8 {
    u8::try_from(delays.len())
        .unwrap_or(RETRY_LIMIT)
        .min(RETRY_LIMIT)
        .saturating_add(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(values: &[u64]) -> Vec<Duration> {
        values
            .iter()
            .map(|value| Duration::from_millis(*value))
            .collect()
    }

    #[test]
    fn the_schedule_sets_one_retry_per_delay_up_to_the_ceiling_and_clamps_each_delay() {
        let cases: [(&[u64], &[u64]); 7] = [
            (&[250, 500, 1000], &[250, 500, 1000]),
            (&[], &[]),
            (&[40], &[40]),
            (&[40, 80], &[40, 80]),
            (&[1, 2, 3, 4, 5], &[1, 2, 3, 4, 5]),
            (&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10], &[1, 2, 3, 4, 5, 6, 7, 8]),
            (&[60_000, 5, 31_000], &[30_000, 5, 30_000]),
        ];
        for (configured, expected) in cases {
            let delays = turn_retry_delays(&ms(configured));
            assert_eq!(delays, ms(expected), "configured {configured:?}");
            assert_eq!(
                usize::from(max_attempts(&delays)),
                expected.len() + 1,
                "configured {configured:?}"
            );
        }
    }
}
