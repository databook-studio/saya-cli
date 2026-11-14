//! The resume range check: a `206 Partial Content` is only accepted when
//! the server confirms it served the offset the resume asked for.

use super::super::download_error::DownloadError;
use super::super::transport::WireResponse;

/// Verifies a 206 against the offset the resume asked for.
pub(super) fn verify_range(
    url: &str,
    response: &WireResponse,
    expected: Option<u64>,
) -> Result<(), DownloadError> {
    let expected = expected.unwrap_or_default();
    let mismatch = |served: String| DownloadError::RangeMismatch {
        url: url.to_owned(),
        expected,
        served,
    };
    let served = response
        .content_range
        .as_deref()
        .ok_or_else(|| mismatch("(absent)".to_owned()))?;
    let served_start = served
        .strip_prefix("bytes ")
        .and_then(|rest| rest.split('-').next())
        .and_then(|offset| offset.parse::<u64>().ok());
    match served_start {
        Some(start) if start == expected => Ok(()),
        _ => Err(mismatch(served.to_owned())),
    }
}
