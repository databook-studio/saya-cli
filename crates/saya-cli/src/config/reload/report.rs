//! The pure report text for a reload plan: identical on every surface, and
//! only redacted labels can reach it (the values come from the diagnostics).

use super::plan::{ChangeLine, ReloadPlan};
use serde_json::Value;

impl ReloadPlan {
    /// Pure report text, identical on every surface.
    pub(crate) fn render(&self) -> String {
        if self.is_empty() {
            return unchanged(&self.ignored_project_overrides);
        }
        let mut out = format!(
            "Reloaded configuration: {} applied, {} re-seeded, {} pending.",
            self.applied.len(),
            self.reseeded.len(),
            self.pending.len()
        );
        section(&mut out, "Applied from the next turn:", &self.applied);
        section(&mut out, "Updated for this session:", &self.reseeded);
        section(&mut out, "Not applied in this session:", &self.pending);
        ignored_line(&mut out, &self.ignored_project_overrides);
        out
    }
}

fn unchanged(ignored: &[String]) -> String {
    let mut out = String::from("Configuration is unchanged.");
    ignored_line(&mut out, ignored);
    out
}

fn section(out: &mut String, title: &str, lines: &[ChangeLine]) {
    if lines.is_empty() {
        return;
    }
    out.push('\n');
    out.push_str(title);
    for line in lines {
        out.push_str(&format!(
            "\n  {}: {} -> {}",
            line.key,
            format_value(&line.before),
            format_value(&line.after)
        ));
        if let Some(note) = &line.note {
            out.push_str(&format!(" — {note}"));
        }
    }
}

fn ignored_line(out: &mut String, ignored: &[String]) {
    if ignored.is_empty() {
        return;
    }
    out.push_str(&format!(
        "\nIgnored from the project file: {} (security-critical; see saya config show --explain).",
        ignored.join(", ")
    ));
}

/// Strings render unquoted; everything else as compact JSON.
fn format_value(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}
