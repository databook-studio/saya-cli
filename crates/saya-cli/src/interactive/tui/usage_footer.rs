//! The per-turn footer that surfaces token usage inline in the transcript.
//!
//! This is the always-visible usage surface: cumulative session totals ride
//! along with the per-turn figures the footer already carried, and the
//! context-window utilisation is appended when it can be known. `/usage`
//! remains the detailed breakdown; this line is the glanceable one.
//!
//! Every unknown stays absent: without both a known window and a provider
//! report of the current input there is no honest percentage, and a segment
//! that rendered `0%` for "we do not know" would be a lie, not a default.

use super::usage_totals::UsageTotals;
use saya_agent::{CONTEXT_COMPACT_PERCENT, TokenUsage, context_utilisation_percent};

/// Builds the footer pushed to the transcript after a turn that reported
/// usage: the per-turn counts, the cumulative answering totals (this turn
/// included), and — only when both are known — the last reported input as a
/// share of the model's context window.
///
/// The session segment covers the answering calls only: the extraction call
/// is billed apart in `/usage`, and folding it in here would silently merge
/// the two labelled totals. The context numerator is the last *answering*
/// call's `input_tokens` — the aggregate at the end of a run sums every
/// round's re-sent conversation, so it would inflate the percentage on
/// exactly the tool-heavy turns where the figure matters most.
pub(crate) fn transcript_footer(
    turn: &TokenUsage,
    session: &UsageTotals,
    last_answering_input: Option<u64>,
    window: Option<u64>,
) -> String {
    let mut footer = format!(
        "{} tokens in · {} tokens out",
        turn.input_tokens, turn.output_tokens
    );
    footer.push_str(&format!(
        " · session {} in / {} out",
        session.input_tokens, session.output_tokens
    ));
    if let (Some(input), Some(window)) = (last_answering_input, window)
        && let Some(percent) = context_utilisation_percent(Some(input), Some(window))
    {
        footer.push_str(&format!(
            " · ctx {}% of {}",
            percent,
            compact_tokens(window)
        ));
    }
    footer
}

/// Builds the one-shot context-window warning pushed after the footer when
/// utilisation crosses the warn threshold upward. States the percentage, the
/// window, and what fires at the compact threshold — the user learns the
/// behaviour before it happens, not when it fires.
///
/// Returns `None` below the threshold; the armed/fired crossing lives in the
/// caller (`SessionState::context_warned`), so this stays a pure renderer.
pub(crate) fn context_warn_notice(percent: u64, window: u64) -> Option<String> {
    if percent < saya_agent::CONTEXT_WARN_PERCENT {
        return None;
    }
    Some(format!(
        "Context is at {percent}% of {} tokens. At {CONTEXT_COMPACT_PERCENT}% saya will summarise older turns to keep going; /clear resets the working memory now if you prefer.",
        compact_tokens(window)
    ))
}

/// Renders a token count compactly for the footer: exact below a thousand,
/// three-significant-figure shorthand above it. The shorthand rounds for
/// display only — the session totals beside it stay exact.
fn compact_tokens(tokens: u64) -> String {
    let (divisor, suffix) = if tokens >= 1_000_000 {
        (1_000_000.0, "M")
    } else if tokens >= 1_000 {
        (1_000.0, "k")
    } else {
        return tokens.to_string();
    };
    let rendered = format!("{:.1}{suffix}", tokens as f64 / divisor);
    rendered.replace(".0", "")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn totals(input: u64, output: u64) -> UsageTotals {
        UsageTotals {
            input_tokens: input,
            output_tokens: output,
            ..UsageTotals::default()
        }
    }

    /// The footer keeps the original per-turn wording first, then carries the
    /// cumulative session totals inline — the glanceable answer to "what has
    /// this session cost" that previously required typing `/usage`.
    #[test]
    fn the_footer_keeps_the_turn_wording_and_adds_session_totals() {
        let footer = transcript_footer(&TokenUsage::new(300, 100), &totals(1300, 600), None, None);
        assert_eq!(
            footer, "300 tokens in · 100 tokens out · session 1300 in / 600 out",
            "the footer must carry the cumulative session totals inline"
        );
    }

    /// With a known window and a provider report of the current input, the
    /// footer shows the utilisation. The window renders compactly but the
    /// percentage stays exact to the display precision.
    #[test]
    fn context_utilisation_renders_against_a_compact_window() {
        let footer = transcript_footer(
            &TokenUsage::new(64_000, 20),
            &totals(64_000, 20),
            Some(64_000),
            Some(128_000),
        );
        assert!(
            footer.ends_with("· ctx 50% of 128k"),
            "ctx must show the last reported input against the known window: {footer}"
        );
    }

    /// No window, or no provider report of the input, means no ctx figure —
    /// "we do not know" must render as absence, never as `0%`.
    #[test]
    fn context_utilisation_is_absent_without_window_or_input_report() {
        let no_window = transcript_footer(
            &TokenUsage::new(64_000, 20),
            &totals(64_000, 20),
            Some(64_000),
            None,
        );
        assert!(
            !no_window.contains("ctx"),
            "no window means no ctx figure: {no_window}"
        );

        let no_report = transcript_footer(
            &TokenUsage::new(64_000, 20),
            &totals(64_000, 20),
            None,
            Some(128_000),
        );
        assert!(
            !no_report.contains("ctx"),
            "no provider report means no ctx figure: {no_report}"
        );
    }

    /// The window shorthand stays three significant figures and never rounds
    /// a distinct window up into another model's window.
    #[test]
    fn window_shorthand_keeps_three_significant_figures() {
        assert_eq!(compact_tokens(131_072), "131.1k");
        assert_eq!(compact_tokens(128_000), "128k");
        assert_eq!(compact_tokens(1_048_576), "1M");
        assert_eq!(compact_tokens(999), "999");
    }

    /// A *reported* zero input is a fact the provider gave us, so `0%` is the
    /// honest rendering — the same discipline as a reported zero cache rate,
    /// which is a cold cache, not an unknown.
    #[test]
    fn a_reported_zero_input_renders_zero_percent_not_absent() {
        let footer = transcript_footer(
            &TokenUsage::new(0, 100),
            &totals(0, 100),
            Some(0),
            Some(128_000),
        );
        assert!(
            footer.contains("ctx 0% of 128k"),
            "a reported zero input is a fact, so 0% renders: {footer}"
        );
    }

    /// The notice states the percentage, the window, and what happens at the
    /// compact threshold — the user learns the behaviour before it fires.
    /// The trigger and the text both derive from the shared thresholds, so a
    /// later slice cannot drift one without the other.
    #[test]
    fn the_notice_derives_from_the_shared_thresholds() {
        let notice = context_warn_notice(72, 200_000).expect("a notice renders");
        assert!(
            notice.contains("72%"),
            "the notice states the percentage: {notice}"
        );
        assert!(
            notice.contains("200k"),
            "the notice states the window: {notice}"
        );
        assert!(
            notice.contains(&saya_agent::CONTEXT_COMPACT_PERCENT.to_string()),
            "the notice names the compact threshold: {notice}"
        );
    }
}
