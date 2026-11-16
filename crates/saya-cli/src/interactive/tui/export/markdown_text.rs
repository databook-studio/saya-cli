//! Pure text handling for the Markdown report: value neutralisation, the
//! self-protecting SQL fence, and the UTC timestamp. No I/O — the report
//! builder in `markdown.rs` calls these.

/// Cells longer than this are cut with `…` so one wide value cannot blow up
/// the document or the table layout.
const MAX_CELL_CHARS: usize = 200;

/// One table value: the raw value is cut to [`MAX_CELL_CHARS`] first, then
/// control characters become spaces, `|` is escaped so the cell cannot break
/// the table, and `\` plus the Markdown structural characters are escaped so
/// no link, image, HTML, or autolink can form.
pub(super) fn neutralize_cell(value: &serde_json::Value) -> String {
    let raw = match value {
        serde_json::Value::Null => String::new(),
        serde_json::Value::String(text) => text.clone(),
        other => other.to_string(),
    };
    neutralize_text(&raw)
}

pub(super) fn neutralize_text(raw: &str) -> String {
    let cut = match raw.char_indices().nth(MAX_CELL_CHARS) {
        Some((index, _)) => &raw[..index],
        None => raw,
    };
    let mut out = String::new();
    for ch in cut.chars() {
        match ch {
            c if c.is_control() => out.push(' '),
            '\\' => out.push_str("\\\\"),
            '|' => out.push_str("\\|"),
            '`' | '<' | '>' | '[' | ']' | '(' | ')' | '!' => {
                out.push('\\');
                out.push(ch);
            }
            other => out.push(other),
        }
    }
    if cut.len() < raw.len() {
        out.push('…');
    }
    out
}

/// The fenced SQL block: the fence is always longer than any backtick run
/// inside, so the SQL cannot close its own block; the SQL goes in verbatim
/// except that control characters other than `\n` and `\t` are replaced.
pub(super) fn sql_block(sql: &str) -> String {
    let sanitized: String = sql
        .chars()
        .map(|c| match c {
            '\n' | '\t' => c,
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    let mut longest = 0usize;
    let mut run = 0usize;
    for ch in sanitized.chars() {
        run = if ch == '`' { run + 1 } else { 0 };
        longest = longest.max(run);
    }
    let fence = "`".repeat((longest + 1).max(3));
    format!("{fence}sql\n{sanitized}\n{fence}\n\n")
}

/// `YYYY-MM-DD HH:MM:SS UTC` from unix milliseconds — no external date
/// dependency.
pub(super) fn utc_timestamp(unix_ms: i64) -> String {
    let total_seconds = unix_ms.div_euclid(1000);
    let (year, month, day) = civil_from_days(total_seconds.div_euclid(86_400));
    let today = total_seconds.rem_euclid(86_400);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02} UTC",
        today / 3600,
        (today % 3600) / 60,
        today % 60
    )
}

/// Howard Hinnant's `civil_from_days`: days since 1970-01-01 to
/// (year, month, day), UTC.
fn civil_from_days(days_since_epoch: i64) -> (i64, u32, u32) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_097) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    ((if m <= 2 { y + 1 } else { y }), m as u32, d as u32)
}
