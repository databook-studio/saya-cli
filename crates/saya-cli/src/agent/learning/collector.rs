#![allow(dead_code)] // The runtime consumes this pure collector in the following integration packet.

use super::turn_record::MAX_PROMPT_BYTES;
use saya_types::MAX_TEXT_CHARS;

pub(crate) const MAX_USER_NOTES_PER_TURN: usize = 4;

/// Keeps bounded complete assertion sentences exactly as the user wrote them.
pub(crate) fn collect_assertion_sentences(prompt: &str) -> Vec<String> {
    if prompt.len() > MAX_PROMPT_BYTES {
        return Vec::new();
    }
    let mut notes = Vec::new();
    let mut start = 0;
    for (index, ch) in prompt.char_indices() {
        if matches!(ch, '.' | '!' | '?') {
            let end = index + ch.len_utf8();
            if prompt[end..].chars().next().is_none_or(char::is_whitespace) {
                let sentence = prompt[start..end].trim();
                if ch != '?' && !sentence.is_empty() && sentence.chars().count() <= MAX_TEXT_CHARS {
                    notes.push(sentence.to_owned());
                    if notes.len() == MAX_USER_NOTES_PER_TURN {
                        return notes;
                    }
                }
                start = end;
            }
        }
    }
    notes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_four_complete_assertions_verbatim_and_skips_questions() {
        let prompt = "Orders is one row per order. Refunds stay separate! Is orders joined? Keep the exact date. Another assertion. Last sentence.";
        assert_eq!(
            collect_assertion_sentences(prompt),
            [
                "Orders is one row per order.",
                "Refunds stay separate!",
                "Keep the exact date.",
                "Another assertion."
            ]
        );
    }

    #[test]
    fn refuses_oversized_prompt_and_sentence() {
        assert!(collect_assertion_sentences(&"x".repeat(MAX_PROMPT_BYTES + 1)).is_empty());
        assert!(
            collect_assertion_sentences(&format!("{}.", "x".repeat(MAX_TEXT_CHARS + 1))).is_empty()
        );
    }
}
