use crate::cli::CodingCommand;
use serde_json::Value;
#[path = "status.rs"]
mod status;
use status::show;

/// Never print more than this many changed files; the diff itself stays in
/// the structured result the owner persisted.
const MAX_LISTED_CHANGES: usize = 50;

/// Shapes the review and apply results for people. Every other command, and
/// any result whose shape is not the expected one, keeps its JSON so nothing
/// the owner reported is hidden.
pub(crate) fn render_coding_result(command: &CodingCommand, value: &Value) -> String {
    let shaped = match command {
        CodingCommand::Review { id, candidate, .. }
        | CodingCommand::ReviewExisting { id, candidate, .. } => review(id, candidate, value),
        CodingCommand::Apply { candidate, .. } => apply(candidate, value),
        CodingCommand::Setup { .. } => setup(value),
        CodingCommand::Worktrees { id } => worktrees(id, value),
        CodingCommand::Show { .. } => show(value),
        CodingCommand::Reconcile { .. } => reconcile(value),
        _ => None,
    };
    shaped.unwrap_or_else(|| value.to_string())
}

fn review(id: &str, candidate: &str, value: &Value) -> Option<String> {
    let changes = value.get("changes")?.as_array()?;
    let digest = value.get("source_sha256")?.as_str()?;
    let mut lines = vec![
        format!("Review of candidate {candidate} for job {id}"),
        format!("Source digest: {digest}"),
        format!("{} changed files", changes.len()),
    ];
    for change in changes.iter().take(MAX_LISTED_CHANGES) {
        lines.push(change_line(change)?);
    }
    if changes.len() > MAX_LISTED_CHANGES {
        lines.push(format!(
            "  … {} more not listed",
            changes.len() - MAX_LISTED_CHANGES
        ));
    }
    lines.push(format!(
        "Apply: saya job apply {id} {candidate} --source-sha256 {digest} --approve"
    ));
    lines.push(
        "Apply re-checks that the candidate and your original tree are unchanged; if either moved, review again."
            .into(),
    );
    Some(lines.join("\n"))
}

fn change_line(change: &Value) -> Option<String> {
    let kind = change.get("kind")?.as_str()?.to_lowercase();
    let path = change.get("path")?.as_str()?;
    let bytes = change.get("bytes")?.as_u64()?;
    let size = if bytes == 0 {
        String::new()
    } else {
        format!(" ({bytes} bytes)")
    };
    Some(format!("  {kind:<8}  {path}{size}"))
}

fn apply(candidate: &str, value: &Value) -> Option<String> {
    if value.get("applied")?.as_bool() != Some(true) {
        return None;
    }
    let digest = value.get("source_sha256")?.as_str()?;
    let short: String = digest.chars().take(12).collect();
    Some(format!(
        "Applied candidate {candidate} to the original checkout (source {short})."
    ))
}

fn text_of(value: &Value) -> String {
    value
        .as_str()
        .map_or_else(|| value.to_string(), str::to_owned)
}

fn setup(value: &Value) -> Option<String> {
    let steps = value.get("steps")?.as_array()?;
    let blocked = steps
        .iter()
        .filter(|step| step.get("status").and_then(Value::as_str) != Some("ok"))
        .count();
    let headline = match (value.get("ready")?.as_bool()?, blocked) {
        (true, _) => "Job setup: ready".to_owned(),
        (false, 1) => "Job setup: 1 step needs action".to_owned(),
        (false, count) => format!("Job setup: {count} steps need action"),
    };
    let mut lines = vec![headline];
    for step in steps {
        let check = step.get("check")?.as_str()?;
        match step.get("action").and_then(Value::as_str) {
            Some(action) => lines.push(format!("  [blocked]  {check}: {action}")),
            None => lines.push(format!("  [ok]       {check}")),
        }
    }
    lines.push("Nothing was installed, enabled or written.".into());
    Some(lines.join("\n"))
}

fn worktrees(id: &str, value: &Value) -> Option<String> {
    let checkouts = value.get("checkouts")?.as_array()?;
    let mut lines = vec![format!(
        "{} managed checkouts for job {id}",
        checkouts.len()
    )];
    for checkout in checkouts {
        lines.push(format!(
            "  {}  {}",
            checkout.get("checkout_id")?.as_str()?,
            text_of(checkout.get("lifecycle")?)
        ));
    }
    if value
        .get("next_cursor")
        .is_some_and(|cursor| !cursor.is_null())
    {
        lines.push("  … more checkouts exist; list again after the last id".into());
    }
    Some(lines.join("\n"))
}

fn reconcile(value: &Value) -> Option<String> {
    let job = value.get("job_id")?.as_str()?;
    if let Some(done) = value.get("reconciled").and_then(Value::as_array) {
        let sequences: Vec<String> = done.iter().map(text_of).collect();
        return Some(format!(
            "Reconciled {} effects for job {job} (reservations {}); nothing was refunded.\nNext: saya job resume {job} --approve",
            done.len(),
            sequences.join(", ")
        ));
    }
    let effects = value.get("effects")?.as_array()?;
    if effects.is_empty() {
        return Some(format!(
            "Nothing is unresolved for job {job}; use saya job resume {job} --approve."
        ));
    }
    let digest = value.get("plan_sha256")?.as_str()?;
    let mut lines = vec![format!(
        "{} unresolved effects for job {job}",
        effects.len()
    )];
    for effect in effects {
        let state = if effect.get("settled_uncertain")?.as_bool()? {
            "settled uncertain"
        } else {
            "open"
        };
        let charges: Vec<String> = effect
            .get("reserved")?
            .as_array()?
            .iter()
            .map(|charge| {
                Some(format!(
                    "{} {}",
                    charge.get("dimension")?.as_str()?,
                    charge.get("reserved")?
                ))
            })
            .collect::<Option<_>>()?;
        lines.push(format!(
            "  reservation {}  {state}  {}",
            effect.get("reservation_sequence")?,
            charges.join(", ")
        ));
    }
    lines.push("This listing changed nothing. Reconciling records each effect as reconciled: its charge stays consumed, nothing is refunded, killed or re-run, and resume still needs its own approval.".into());
    lines.push(format!(
        "Next: saya job reconcile {job} --plan-sha256 {digest} --approve"
    ));
    Some(lines.join("\n"))
}
