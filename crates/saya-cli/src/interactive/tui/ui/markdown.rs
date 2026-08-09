//! Lightweight markdown styling for assistant transcript lines.

use super::theme::{ACCENT, CODE_COLOR};
use ratatui::{
    style::{Color, Modifier, Style},
    text::Span,
};

/// Formats a single transcript view line with lightweight markdown styling for assistant output.
pub(super) fn markdown_spans(line: &str) -> Vec<Span<'static>> {
    let base = Style::default().fg(Color::White);
    let trimmed = line.trim_start();

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
        let mut spans = vec![Span::styled("• ", base.fg(ACCENT))];
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
                    spans.push(Span::styled(mid.to_string(), base.fg(CODE_COLOR)));
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
    use ratatui::style::Modifier;

    #[test]
    fn test_bold_text() {
        let spans = markdown_spans("**bold** text");
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].content.as_ref(), "bold");
        assert!(spans[0].style.add_modifier.contains(Modifier::BOLD));
        assert_eq!(spans[1].content.as_ref(), " text");
        assert!(!spans[1].style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn test_inline_code() {
        let spans = markdown_spans("run `SELECT 1` now");
        assert_eq!(spans.len(), 3);
        assert_eq!(spans[0].content.as_ref(), "run ");
        assert_eq!(spans[1].content.as_ref(), "SELECT 1");
        assert_eq!(spans[1].style.fg, Some(CODE_COLOR));
        assert_eq!(spans[2].content.as_ref(), " now");
    }

    #[test]
    fn test_heading() {
        let spans = markdown_spans("# Heading");
        let combined: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(combined, "Heading");
        assert!(!combined.contains('#'));
        for span in &spans {
            assert!(span.style.add_modifier.contains(Modifier::BOLD));
        }
    }

    #[test]
    fn test_bullet_item() {
        let spans = markdown_spans("- item");
        assert!(!spans.is_empty());
        assert_eq!(spans[0].content.as_ref(), "• ");
        assert_eq!(spans[0].style.fg, Some(ACCENT));
        let rest_combined: String = spans[1..].iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(rest_combined, "item");
    }

    #[test]
    fn test_plain_text() {
        let spans = markdown_spans("plain text with no markers");
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content.as_ref(), "plain text with no markers");
    }

    #[test]
    fn test_unclosed_bold() {
        let spans = markdown_spans("unclosed **bold");
        let combined: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(combined, "unclosed **bold");
        assert!(combined.contains("**"));
    }
}
