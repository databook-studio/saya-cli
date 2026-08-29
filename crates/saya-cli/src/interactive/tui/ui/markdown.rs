//! Lightweight markdown styling for assistant transcript lines.

use super::theme::{accent, code_color, secondary};
use ratatui::{
    style::{Color, Modifier, Style},
    text::Span,
};

/// Formats a single transcript view line with lightweight markdown styling for assistant output.
fn base_style() -> Style {
    Style::default().fg(Color::White)
}

/// Styles one assistant line, tracking ``` fence state across consecutive
/// lines of the block (`fence` is owned by the caller's render loop).
pub(super) fn markdown_spans_fenced(line: &str, fence: &mut bool) -> Vec<Span<'static>> {
    let base = base_style();
    let trimmed = line.trim_start();

    if trimmed.starts_with("```") {
        *fence = !*fence;
        return vec![Span::styled(
            "···".to_string(),
            Style::default().fg(secondary()),
        )];
    }
    if *fence {
        // Literal code: no inline markdown parsing inside fences.
        return vec![Span::styled(
            line.to_string(),
            Style::default().fg(code_color()),
        )];
    }

    if let Some(rest) = trimmed
        .strip_prefix("### ")
        .or_else(|| trimmed.strip_prefix("## "))
        .or_else(|| trimmed.strip_prefix("# "))
    {
        let heading_style = base.add_modifier(Modifier::BOLD);
        inline_spans(rest, heading_style)
    } else if let Some(rest) = trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))
    {
        let mut spans = vec![Span::styled("• ", base.fg(accent()))];
        spans.extend(inline_spans(rest, base));
        spans
    } else {
        inline_spans(line, base)
    }
}

/// Parses inline bold (`**bold**`) and inline code (`` `code` ``) formatting.
fn inline_spans(text: &str, base: Style) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut plain_buf = String::new();
    let mut rem = text;

    while !rem.is_empty() {
        let pos_bold = rem.find("**");
        let pos_code = rem.find('`');

        match (pos_bold, pos_code) {
            (Some(b), Some(c)) if b < c => {
                if let Some(close_rel) = rem[b + 2..].find("**") {
                    plain_buf.push_str(&rem[..b]);
                    if !plain_buf.is_empty() {
                        spans.push(Span::styled(std::mem::take(&mut plain_buf), base));
                    }
                    let mid = &rem[b + 2..b + 2 + close_rel];
                    spans.push(Span::styled(
                        mid.to_string(),
                        base.add_modifier(Modifier::BOLD),
                    ));
                    rem = &rem[b + 2 + close_rel + 2..];
                } else {
                    plain_buf.push_str(&rem[..b + 2]);
                    rem = &rem[b + 2..];
                }
            }
            (Some(b), None) => {
                if let Some(close_rel) = rem[b + 2..].find("**") {
                    plain_buf.push_str(&rem[..b]);
                    if !plain_buf.is_empty() {
                        spans.push(Span::styled(std::mem::take(&mut plain_buf), base));
                    }
                    let mid = &rem[b + 2..b + 2 + close_rel];
                    spans.push(Span::styled(
                        mid.to_string(),
                        base.add_modifier(Modifier::BOLD),
                    ));
                    rem = &rem[b + 2 + close_rel + 2..];
                } else {
                    plain_buf.push_str(&rem[..b + 2]);
                    rem = &rem[b + 2..];
                }
            }
            (_pos_b, Some(c)) => {
                if let Some(close_rel) = rem[c + 1..].find('`') {
                    plain_buf.push_str(&rem[..c]);
                    if !plain_buf.is_empty() {
                        spans.push(Span::styled(std::mem::take(&mut plain_buf), base));
                    }
                    let mid = &rem[c + 1..c + 1 + close_rel];
                    spans.push(Span::styled(mid.to_string(), base.fg(code_color())));
                    rem = &rem[c + 1 + close_rel + 1..];
                } else {
                    plain_buf.push_str(&rem[..c + 1]);
                    rem = &rem[c + 1..];
                }
            }
            (None, None) => {
                plain_buf.push_str(rem);
                rem = "";
            }
        }
    }

    if !plain_buf.is_empty() {
        spans.push(Span::styled(plain_buf, base));
    }

    if spans.is_empty() {
        vec![Span::styled(String::new(), base)]
    } else {
        spans
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bold_text() {
        let mut fence = false;
        let spans = markdown_spans_fenced("**bold** text", &mut fence);
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].content.as_ref(), "bold");
        assert!(spans[0].style.add_modifier.contains(Modifier::BOLD));
        assert_eq!(spans[1].content.as_ref(), " text");
        assert!(!spans[1].style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn test_inline_code() {
        let mut fence = false;
        let spans = markdown_spans_fenced("run `SELECT 1` now", &mut fence);
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0].content.as_ref(), "run ");
        assert_eq!(spans[1].content.as_ref(), "SELECT 1");
        assert_eq!(spans[1].style.fg, Some(code_color()));
        assert_eq!(spans[2].content.as_ref(), " now");
    }

    #[test]
    fn test_heading() {
        let mut fence = false;
        let spans = markdown_spans_fenced("# Heading", &mut fence);
        let combined: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(combined, "Heading");
        assert!(!combined.contains('#'));
        for span in &spans {
            assert!(span.style.add_modifier.contains(Modifier::BOLD));
        }
    }

    #[test]
    fn test_bullet_item() {
        let mut fence = false;
        let spans = markdown_spans_fenced("- item", &mut fence);
        assert!(!spans.is_empty());
        assert_eq!(spans[0].content.as_ref(), "• ");
        assert_eq!(spans[0].style.fg, Some(accent()));
        let rest_combined: String = spans[1..].iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(rest_combined, "item");
    }

    #[test]
    fn test_plain_text() {
        let mut fence = false;
        let spans = markdown_spans_fenced("plain text with no markers", &mut fence);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content.as_ref(), "plain text with no markers");
    }

    #[test]
    fn fenced_blocks_render_literally_between_markers() {
        let mut fence = false;
        assert_eq!(
            markdown_spans_fenced("```sql", &mut fence)[0]
                .content
                .as_ref(),
            "···"
        );
        assert!(fence);
        let mut fence_body = true;
        let body = markdown_spans_fenced("SELECT x = '**not bold**'", &mut fence_body);
        let combined: String = body.iter().map(|span| span.content.as_ref()).collect();
        assert_eq!(
            combined, "SELECT x = '**not bold**'",
            "inside a fence nothing is parsed"
        );
        assert!(body[0].style.fg == Some(code_color()));
        assert_eq!(
            markdown_spans_fenced("```", &mut fence)[0].content.as_ref(),
            "···"
        );
        assert!(!fence);
        let after = markdown_spans_fenced("plain again", &mut fence);
        assert_eq!(after[0].content.as_ref(), "plain again");
    }

    #[test]
    fn test_unclosed_bold() {
        let mut fence = false;
        let spans = markdown_spans_fenced("unclosed **bold", &mut fence);
        let combined: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(combined, "unclosed **bold");
        assert!(combined.contains("**"));
    }
}
