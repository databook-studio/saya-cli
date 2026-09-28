//! Type inference for the staged-source preview: per column, the narrowest
//! type every sampled non-empty value can claim. Stored data is never
//! converted — the table stays VARCHAR; this only labels the preview.
//!
//! Strict shapes only: leading-zero numerics ("007"), non-ISO dates, and
//! anything with stray whitespace fall back to text. Booleans are exactly
//! `true`/`false`. Dates are exactly ISO `YYYY-MM-DD`. Timestamps are RFC
//! 3339 (offset required) or a naive ISO `YYYY-MM-DD[T ]HH:MM:SS[.frac]`.

use chrono::{DateTime, NaiveDate};

/// How many leading data rows inference may look at.
pub const INFER_SAMPLE_ROWS: usize = 1_000;

/// The preview's inferred column type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InferredType {
    Integer,
    Decimal,
    Boolean,
    Date,
    Timestamp,
    Text,
}

const INTEGER: u16 = 1;
const DECIMAL: u16 = 2;
const BOOLEAN: u16 = 4;
const DATE: u16 = 8;
const TIMESTAMP: u16 = 16;

/// Intersects the candidate masks across the sample and picks the narrowest
/// survivor. A column with no sampled non-empty value has no evidence and
/// stays text.
pub(super) fn infer(rows: &[Vec<String>], column_count: usize) -> Vec<InferredType> {
    let sample = rows.len().min(INFER_SAMPLE_ROWS);
    (0..column_count)
        .map(|column| infer_column(rows, sample, column))
        .collect()
}

fn infer_column(rows: &[Vec<String>], sample: usize, column: usize) -> InferredType {
    let mut candidates = u16::MAX;
    let mut seen = false;
    for row in rows.iter().take(sample) {
        let value = row.get(column).map(String::as_str).unwrap_or("");
        if value.is_empty() {
            continue;
        }
        seen = true;
        candidates &= classify(value);
    }
    if !seen {
        return InferredType::Text;
    }
    if candidates & INTEGER != 0 {
        InferredType::Integer
    } else if candidates & DECIMAL != 0 {
        InferredType::Decimal
    } else if candidates & BOOLEAN != 0 {
        InferredType::Boolean
    } else if candidates & DATE != 0 {
        InferredType::Date
    } else if candidates & TIMESTAMP != 0 {
        InferredType::Timestamp
    } else {
        InferredType::Text
    }
}

/// The types one value could claim. Integer values are also decimal values —
/// an integer casts cleanly to a decimal; the reverse does not hold, so a
/// column mixing "1" and "1.5" narrows to decimal.
fn classify(value: &str) -> u16 {
    let mut mask = 0;
    if is_integer(value) {
        mask |= INTEGER | DECIMAL;
    } else if is_decimal(value) {
        mask |= DECIMAL;
    }
    if value == "true" || value == "false" {
        mask |= BOOLEAN;
    }
    if is_iso_date(value) {
        mask |= DATE;
    }
    if is_timestamp(value) {
        mask |= TIMESTAMP;
    }
    mask
}

fn is_integer(value: &str) -> bool {
    let digits = value.strip_prefix(['+', '-']).unwrap_or(value);
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return false;
    }
    if digits.len() > 1 && digits.starts_with('0') {
        return false;
    }
    value.parse::<i64>().is_ok()
}

fn is_decimal(value: &str) -> bool {
    let (mantissa, exponent) = value
        .split_once(['e', 'E'])
        .map_or((value, None), |(m, e)| (m, Some(e)));
    if let Some(exponent) = exponent {
        let digits = exponent.strip_prefix(['+', '-']).unwrap_or(exponent);
        if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
            return false;
        }
    }
    let mantissa = mantissa.strip_prefix(['+', '-']).unwrap_or(mantissa);
    let (int_part, frac_part) = match mantissa.split_once('.') {
        Some((int_part, frac_part)) => (int_part, Some(frac_part)),
        None => (mantissa, None),
    };
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
    let well_formed = match (int_part.is_empty(), frac_part) {
        // ".5" — no integer part at all.
        (true, Some(frac_part)) => digits(frac_part),
        // "1.5" — a dot demands a fractional part ("1." stays text).
        (false, Some(frac_part)) => digits(int_part) && digits(frac_part),
        // Bare digits also qualify as decimal candidates (see `classify`).
        (false, None) => digits(int_part),
        _ => false,
    };
    if !well_formed || (int_part.len() > 1 && int_part.starts_with('0')) {
        return false;
    }
    value.parse::<f64>().is_ok_and(|parsed| parsed.is_finite())
}

fn is_iso_date(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || !(bytes[..4].iter().all(u8::is_ascii_digit)
            && bytes[5..7].iter().all(u8::is_ascii_digit)
            && bytes[8..10].iter().all(u8::is_ascii_digit))
    {
        return false;
    }
    NaiveDate::parse_from_str(value, "%Y-%m-%d").is_ok()
}

fn is_timestamp(value: &str) -> bool {
    DateTime::parse_from_rfc3339(value).is_ok() || is_naive_iso_datetime(value)
}

fn is_naive_iso_datetime(value: &str) -> bool {
    let Some((date, rest)) = value.split_at_checked(10) else {
        return false;
    };
    if !is_iso_date(date) {
        return false;
    }
    let Some(rest) = rest.strip_prefix(['T', ' ']) else {
        return false;
    };
    let bytes = rest.as_bytes();
    let digits = |range: std::ops::Range<usize>| bytes[range].iter().all(u8::is_ascii_digit);
    if bytes.len() < 8
        || !digits(0..2)
        || bytes[2] != b':'
        || !digits(3..5)
        || bytes[5] != b':'
        || !digits(6..8)
    {
        return false;
    }
    let hour = rest[0..2].parse::<u32>().unwrap_or(24);
    let minute = rest[3..5].parse::<u32>().unwrap_or(60);
    let second = rest[6..8].parse::<u32>().unwrap_or(60);
    if hour > 23 || minute > 59 || second > 59 {
        return false;
    }
    match rest.get(8..) {
        None => false,
        Some("") => true,
        Some(fraction) => {
            fraction.starts_with('.')
                && fraction.len() > 1
                && fraction[1..].bytes().all(|byte| byte.is_ascii_digit())
        }
    }
}
