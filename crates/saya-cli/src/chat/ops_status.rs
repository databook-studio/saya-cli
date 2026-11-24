//! `/status`: one panel, five sections, and the notes the deprecated spellings
//! state.
//!
//! The panel is a value, not a search: every fact it shows is one the session
//! already holds — its id, its record, its run link, the provider and model in
//! force, this session's token totals, the bound workspace and host lane, the
//! instruction sources the next request would load, and the ceilings in force.
//! Nothing here probes a provider, a database, or the network; `/doctor` is the
//! command that checks, and it stays separate.
//!
//! `/usage`, `/workspace` and `/context` are one-release aliases: each renders
//! the one section it names, plus a one-line note the first time this session
//! says it. `/history` and `/rotate` are aliases of `/sessions` and of
//! `/compact --new-session`, and their notes are [`Note`]s too.

pub(crate) use super::ops_status_facts::{StatusFacts, instructions_answer, panel_text};
pub(crate) use super::ops_status_notes::{Note, Section, StatusNotes, with_note};

/// One section of the panel: its heading and the facts under it, one per line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SectionView {
    section: Section,
    lines: Vec<String>,
}

impl SectionView {
    /// The heading alone.
    pub(crate) fn heading(&self) -> &'static str {
        self.section.heading()
    }

    /// The section as it prints: the heading, then every fact indented.
    pub(crate) fn render(&self) -> String {
        let mut out = String::from(self.section.heading());
        for line in &self.lines {
            out.push('\n');
            if !line.is_empty() {
                out.push_str("  ");
            }
            out.push_str(line);
        }
        out
    }
}

/// The whole panel: exactly the five sections, in [`Section::ALL`] order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StatusPanel {
    views: [SectionView; Section::ALL.len()],
}

impl StatusPanel {
    /// The panel the facts describe, one fact per line under each heading.
    pub(crate) fn build(facts: &StatusFacts) -> Self {
        let views = Section::ALL.map(|section| SectionView {
            section,
            lines: lines_of(section, facts),
        });
        Self { views }
    }

    /// Every section, in print order.
    pub(crate) fn sections(&self) -> &[SectionView] {
        &self.views
    }

    /// The panel as it prints: a heading per section, a blank line between.
    pub(crate) fn render(&self) -> String {
        self.views
            .iter()
            .map(SectionView::render)
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    /// One section as it prints; a section the panel does not carry is empty.
    pub(crate) fn render_section(&self, section: Section) -> String {
        let index = section.index();
        self.views
            .get(index)
            .filter(|view| view.section == section)
            .map(SectionView::render)
            .unwrap_or_default()
    }
}

/// The facts one section shows, in the section's own words.
fn lines_of(section: Section, facts: &StatusFacts) -> Vec<String> {
    match section {
        Section::Session => vec![
            fact("Id", Some(&facts.session_id), "no session id"),
            fact(
                "Record size",
                facts.record.as_deref(),
                "unknown — no saved record has been measured",
            ),
            fact("Run", facts.run.as_deref(), "no run link"),
        ],
        Section::Model => vec![
            fact("Provider", Some(&facts.provider), "none"),
            fact("Model", Some(&facts.model), "none"),
            fact(
                "Context use",
                facts.context_use.as_deref(),
                "unknown — no answering call has reported a window this session",
            ),
            fact("Tokens", Some(&facts.tokens), "not reported"),
            fact(
                "Cost",
                facts.cost.as_deref(),
                "unknown — no answering call has reported a cost this session",
            ),
        ],
        Section::Workspace => vec![
            fact(
                "Root",
                facts.workspace_root.as_deref(),
                "none — writes and downloads are unavailable",
            ),
            format!(
                "Host lane: {}",
                if facts.host_lane {
                    "composed"
                } else {
                    "not composed"
                }
            ),
        ],
        Section::Instructions => vec![fact(
            "Sources",
            facts.instructions.as_deref(),
            "unknown — the loadable instruction sources were not read",
        )],
        Section::Limits => {
            let mut lines = if facts.limits.is_empty() {
                vec!["no [limits.job] guards declared".to_owned()]
            } else {
                facts.limits.clone()
            };
            lines.extend(facts.database_budget.iter().cloned());
            lines
        }
    }
}

/// One `Label: value` line; a multi-line value keeps every line and its own
/// indentation, so a section reads as one fact per line.
fn fact(label: &str, value: Option<&str>, absent: &str) -> String {
    let value = value.filter(|value| !value.is_empty()).unwrap_or(absent);
    let mut lines = value.lines();
    let mut out = format!("{label}: {}", lines.next().unwrap_or_default());
    for line in lines {
        out.push('\n');
        if !line.is_empty() {
            out.push_str("    ");
        }
        out.push_str(line);
    }
    out
}

#[cfg(test)]
#[path = "ops_status_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "ops_status_window_tests.rs"]
mod window_tests;
