//! Literal-grammar checks for parameter values, written by hand because the
//! crate carries no date or decimal dependency. Each parser is strict: the
//! whole input must match the shape, and the refusal names what was wrong.
//! The parsed text is kept as given — no canonicalization, no coercion.

pub(crate) fn integer(raw: &str) -> Result<i64, &'static str> {
    let digits = raw.strip_prefix('-').unwrap_or(raw);
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("expected an optional '-' followed by digits");
    }
    raw.parse::<i64>()
        .map_err(|_| "integer is outside the 64-bit range")
}

pub(crate) fn boolean(raw: &str) -> Result<bool, &'static str> {
    match raw {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err("must be exactly \"true\" or \"false\""),
    }
}

/// `-?digits[.digits]`: digits before the dot are required, digits after it
/// are optional, and at most 38 digit characters are allowed in total.
pub(crate) fn decimal(raw: &str) -> Result<(), &'static str> {
    let unsigned = raw.strip_prefix('-').unwrap_or(raw);
    let (whole, fraction) = match unsigned.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (unsigned, None),
    };
    if whole.is_empty() {
        return Err("digits are required before any '.'");
    }
    if !whole.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("expected only digits before any '.'");
    }
    if let Some(fraction) = fraction {
        if fraction.is_empty() {
            return Err("digits are required after any '.'");
        }
        if !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err("expected only digits after the '.'");
        }
    }
    let digits = whole.len() + fraction.map_or(0, str::len);
    if digits > 38 {
        return Err("at most 38 digits are allowed");
    }
    Ok(())
}

/// `YYYY-MM-DD`, a real proleptic-Gregorian date: the leap-day rule decides
/// February, so `2024-02-29` parses and `2023-02-29` does not.
pub(crate) fn date(raw: &str) -> Result<(), &'static str> {
    let bytes = raw.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return Err("expected YYYY-MM-DD");
    }
    let year = four_digits(&bytes[..4])?;
    let month = two_digits(&bytes[5..7])?;
    let day = two_digits(&bytes[8..10])?;
    if !(1..=12).contains(&month) {
        return Err("month must be 01-12");
    }
    if day == 0 || day > days_in_month(year, month) {
        return Err("day does not exist in that month");
    }
    Ok(())
}

/// RFC 3339 `full-date "T" full-time`: the offset is mandatory (`Z`, `z`, or
/// `±HH:MM` within the grammar's ±23:59), a space separator is not accepted,
/// and the leap second `:60` is refused — neither chrono nor common engines
/// round-trip it, and a bound value must survive the database.
pub(crate) fn timestamp(raw: &str) -> Result<(), &'static str> {
    let bytes = raw.as_bytes();
    if bytes.len() < 20 {
        return Err("expected an RFC 3339 timestamp with Z or a ±HH:MM offset");
    }
    if bytes[10] != b'T' && bytes[10] != b't' {
        return Err("expected 'T' between the date and the time");
    }
    date(&raw[..10])?;
    let time = &bytes[11..];
    if time.len() < 8 || time[2] != b':' || time[5] != b':' {
        return Err("expected HH:MM:SS after 'T'");
    }
    let hour = two_digits(&time[..2])?;
    let minute = two_digits(&time[3..5])?;
    let second = two_digits(&time[6..8])?;
    if hour > 23 {
        return Err("hour must be 00-23");
    }
    if minute > 59 || second > 59 {
        return Err("minute and second must be 00-59");
    }
    let mut cursor = 8;
    if time.get(cursor) == Some(&b'.') {
        cursor += 1;
        let start = cursor;
        while matches!(time.get(cursor), Some(byte) if byte.is_ascii_digit()) {
            cursor += 1;
        }
        if cursor == start {
            return Err("fractional seconds need at least one digit after the '.'");
        }
    }
    match time.get(cursor) {
        Some(b'Z' | b'z') => cursor += 1,
        Some(b'+' | b'-') => {
            let offset = &time[cursor + 1..];
            if offset.len() != 5 || offset[2] != b':' {
                return Err("offset must be ±HH:MM");
            }
            let offset_hour = two_digits(&offset[..2])?;
            let offset_minute = two_digits(&offset[3..5])?;
            if offset_hour > 23 || offset_minute > 59 {
                return Err("offset must be within ±23:59");
            }
            cursor += 6;
        }
        _ => return Err("missing 'Z' or a ±HH:MM offset"),
    }
    if cursor != time.len() {
        return Err("unexpected characters after the timestamp");
    }
    Ok(())
}

fn four_digits(bytes: &[u8]) -> Result<u32, &'static str> {
    if bytes.len() != 4 || !bytes.iter().all(u8::is_ascii_digit) {
        return Err("expected four digits");
    }
    Ok(bytes
        .iter()
        .fold(0, |acc, byte| acc * 10 + u32::from(byte - b'0')))
}

fn two_digits(bytes: &[u8]) -> Result<u32, &'static str> {
    if bytes.len() != 2 || !bytes.iter().all(u8::is_ascii_digit) {
        return Err("expected two digits");
    }
    Ok(u32::from(bytes[0] - b'0') * 10 + u32::from(bytes[1] - b'0'))
}

fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn is_leap_year(year: u32) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}
