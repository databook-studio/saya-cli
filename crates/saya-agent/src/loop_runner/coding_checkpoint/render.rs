use super::CodingCheckpoint;

impl CodingCheckpoint {
    /// Whole safe facts only. The caller supplies space left after protected
    /// messages and the retained tool groups; omitted history stays explicit.
    pub(in crate::loop_runner) fn render(&self, allowance: usize) -> String {
        if self.facts.is_empty() && self.omitted == 0 {
            return String::new();
        }
        let mut kept = self.facts.as_slice();
        loop {
            let omitted = self
                .omitted
                .saturating_add((self.facts.len() - kept.len()) as u64);
            let body =
                serde_json::json!({"authority":"none","coding_facts":kept,"omitted_facts":omitted})
                    .to_string();
            if body.len() <= allowance {
                return body;
            }
            if kept.is_empty() {
                return String::new();
            }
            kept = &kept[1..];
        }
    }
}
