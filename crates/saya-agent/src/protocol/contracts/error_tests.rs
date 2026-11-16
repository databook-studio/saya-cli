//! The stream-cut error is typed and payload-free. It carries the cut
//! attempt's own clock and the token count estimated from the bytes the stream
//! delivered before it was cut — never provider text — because a cut stream
//! reports no usage and what it had delivered is discarded.

use super::ProviderError;
use crate::ProviderRecoveryReason;

#[test]
fn a_stream_cut_carries_only_the_clock_and_the_estimate() {
    let cut = ProviderError::StreamCut {
        elapsed_secs: 302,
        estimated_tokens: 62_000,
    };
    assert_eq!(
        cut.to_string(),
        "provider stream was cut before its terminator after 302s (~62000 tokens received)"
    );
}

/// A cut that delivered nothing keeps the existing name: `StreamEnded` is
/// still what a plain drop reports, so the retry path keeps its reason.
#[test]
fn stream_ended_without_content_keeps_its_name() {
    assert_eq!(
        ProviderError::StreamEnded.to_string(),
        "provider stream ended before its terminator"
    );
    assert_ne!(
        ProviderError::StreamEnded,
        ProviderError::StreamCut {
            elapsed_secs: 1,
            estimated_tokens: 1,
        },
        "a cut is a distinct cause from a stream that ended with nothing"
    );
}

#[test]
fn a_cut_reports_its_own_recovery_reason() {
    assert_eq!(
        ProviderRecoveryReason::StreamCut,
        ProviderRecoveryReason::StreamCut
    );
    assert_ne!(
        ProviderRecoveryReason::StreamCut,
        ProviderRecoveryReason::StreamEnded
    );
}
