#[derive(Default)]
pub(in crate::loop_runner) struct LegacyAdmission {
    turns: usize,
    tool_calls: usize,
}

impl LegacyAdmission {
    pub(in crate::loop_runner) fn turn_limit(&self, max: Option<usize>) -> bool {
        max.is_some_and(|max| self.turns >= max)
    }

    pub(in crate::loop_runner) fn admit_turn(&mut self) {
        self.turns = self.turns.saturating_add(1);
    }

    pub(in crate::loop_runner) fn tool_limit(&self, max: Option<usize>, requested: usize) -> bool {
        max.is_some_and(|max| self.tool_calls.saturating_add(requested) > max)
    }

    pub(in crate::loop_runner) fn admit_tools(&mut self, requested: usize) {
        self.tool_calls = self.tool_calls.saturating_add(requested);
    }
}
