//! The brief text: the plan state, the step's goal and expected outputs,
//! and the workspace manifest.

use std::cmp::Ordering;
use std::fmt::Write as _;

use saya_types::RunPlan;

use crate::workspace::manifest::ManifestEntry;

/// Renders the brief for `plan`'s step `step`: the plan state, the step's
/// goal and expected outputs, and the workspace manifest. Steps before the
/// current one are marked `earlier` — the driver never claims they
/// completed; that is the resume's (a later slice's) knowledge to bring.
pub(crate) fn render(plan: &RunPlan, step: usize, manifest: &[ManifestEntry]) -> String {
    let mut brief = format!(
        "You are executing one step of an approved run. The plan has {} steps.\n\nPlan:\n",
        plan.steps.len()
    );
    for (index, item) in plan.steps.iter().enumerate() {
        let marker = match index.cmp(&step) {
            Ordering::Less => "earlier",
            Ordering::Equal => "your step",
            Ordering::Greater => "later",
        };
        let _ = writeln!(brief, "{index}. [{marker}] {}", item.goal);
    }
    let spec = &plan.steps[step];
    let _ = writeln!(brief, "\nYour step's goal: {}", spec.goal);
    if spec.expects.is_empty() {
        brief.push_str("Expected outputs: none declared.\n");
    } else {
        brief.push_str("Expected outputs:\n");
        for hint in &spec.expects {
            match hint.description.as_deref() {
                Some(description) => {
                    let _ = writeln!(brief, "- {}: {description}", hint.name);
                }
                None => {
                    let _ = writeln!(brief, "- {}", hint.name);
                }
            }
        }
    }
    brief.push_str("\nWorkspace manifest (path, bytes, sha256):\n");
    if manifest.is_empty() {
        brief.push_str("(the workspace is empty)\n");
    } else {
        for entry in manifest {
            let _ = writeln!(
                brief,
                "- {} ({} bytes, sha256: {})",
                entry.path, entry.size, entry.digest
            );
        }
    }
    brief
}
