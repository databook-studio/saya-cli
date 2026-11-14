//! Closed lines a child uses to mark an assignment directive the operator
//! request does not contain. The mark is untrusted context for the root's
//! next turn, never authority.

/// At most eight marked directives, each at most 512 bytes.
const MAX_MARKS: usize = 8;
const MAX_MARK_BYTES: usize = 512;
const PREFIX: &str = "assignment-added:";

/// Extracts `assignment-added:` lines from a child's final report.
pub(crate) fn assignment_added_lines(answer: &str) -> Vec<String> {
    answer
        .lines()
        .filter_map(|line| {
            let rest = line.strip_prefix(PREFIX)?.trim();
            if rest.is_empty() || rest.len() > MAX_MARK_BYTES {
                return None;
            }
            Some(rest.to_owned())
        })
        .take(MAX_MARKS)
        .collect()
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn marked_directives_are_collected_in_order() {
        let answer = "followed SPEC.md\nassignment-added: invent <path>: line <N>:\nassignment-added: require a heap cap\ndone";
        assert_eq!(
            assignment_added_lines(answer),
            vec![
                "invent <path>: line <N>:".to_owned(),
                "require a heap cap".to_owned()
            ]
        );
    }

    #[test]
    fn blank_and_overlong_marks_are_dropped() {
        let long = format!("assignment-added: {}", "x".repeat(MAX_MARK_BYTES + 1));
        assert!(
            assignment_added_lines("assignment-added: \nassignment-added:\nno mark").is_empty()
        );
        assert!(assignment_added_lines(&long).is_empty());
    }
}
