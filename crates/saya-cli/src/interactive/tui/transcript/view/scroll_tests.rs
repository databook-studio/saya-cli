#[cfg(test)]
mod find_tests {
    use super::super::super::{BlockKind, Transcript};

    fn transcript() -> Transcript {
        let mut t = Transcript::default();
        t.push(BlockKind::User, "show me the orders table");
        t.push(BlockKind::Assistant, "Here is the orders summary.");
        t.push(BlockKind::Error, "column not found: ordrs");
        t
    }

    #[test]
    fn jump_finds_case_insensitive_and_reports_misses() {
        let mut t = transcript();
        assert!(t.jump_to_match("ORDERS", 80, 2));
        assert!(t.scroll_up > 0, "viewport moved to the match");
        assert!(!t.jump_to_match("nonexistent-needle", 80, 2));
        // Following-tail state is untouched by a miss.
        assert!(t.scroll_up > 0);
    }
}
