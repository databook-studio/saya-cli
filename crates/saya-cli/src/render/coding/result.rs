use crate::cli::CodingCommand;
use serde_json::Value;

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
        CodingCommand::Conflicts { id, candidate } => conflicts(id, candidate, value),
        CodingCommand::Worktrees { id } => worktrees(id, value),
        CodingCommand::Show { .. } => show(value),
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
    let checks = value.get("checks")?.as_array()?;
    let mut failing = Vec::new();
    if checks.is_empty() {
        lines.push("No checks are configured for this candidate.".into());
    } else {
        lines.push("Checks:".into());
    }
    for check in checks {
        let profile = check.get("profile")?.as_str()?;
        lines.push(format!("  {profile}: {}", text_of(check.get("outcome")?)));
        if check.get("successful")?.as_bool() != Some(true) {
            failing.push(profile);
        }
    }
    if let Some(delta) = value.get("diagnostic_delta") {
        lines.push(delta_line(delta));
    }
    if value.get("checks_configured")?.as_bool() != Some(true) {
        lines.push("Apply is refused until successful configured checks exist.".into());
    } else if !failing.is_empty() {
        lines.push(format!(
            "Apply will be refused until every check passes (not passing: {}).",
            failing.join(", ")
        ));
    } else {
        lines.push(format!(
            "Apply: /coding apply {id} {candidate} --source-sha256 {digest} --approve"
        ));
        lines.push(
            "Apply re-checks that the candidate and your original tree are unchanged; if either moved, review again."
                .into(),
        );
    }
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

fn delta_line(delta: &Value) -> String {
    if delta.get("available").and_then(Value::as_bool) == Some(true) {
        let count = |key: &str| delta.get(key).and_then(Value::as_u64).unwrap_or(0);
        format!(
            "Diagnostics: {} introduced, {} unchanged, {} resolved",
            count("introduced_count"),
            count("unchanged_count"),
            count("resolved_count")
        )
    } else {
        let reason = delta
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or("unavailable");
        format!("Diagnostics: no comparison ({reason})")
    }
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
        (true, _) => "Coding setup: ready".to_owned(),
        (false, 1) => "Coding setup: 1 step needs action".to_owned(),
        (false, count) => format!("Coding setup: {count} steps need action"),
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

fn conflicts(id: &str, candidate: &str, value: &Value) -> Option<String> {
    match value.get("status")?.as_str()? {
        "not_retained" => Some(format!(
            "No conflict is retained for candidate {candidate} (job {id})."
        )),
        "retained" => {
            let record = value.get("provenance")?.get("record_sha256")?.as_str()?;
            let review = value
                .get("manual_edit")
                .and_then(|edit| edit.get("review_command"))
                .and_then(Value::as_str)
                .unwrap_or("coding review-existing");
            Some(
                [
                    format!("Conflict retained for candidate {candidate} (job {id})"),
                    "This listing changed nothing and grants no authority to resolve; the retained record is revalidated freshly before anything is rebuilt.".into(),
                    "Resolve by choice (rebuilds one fresh candidate, needs approval):".into(),
                    format!("  /coding resolve-conflict {id} {candidate} --choice keep-first --record-sha256 {record} --approve"),
                    "  … or --choice keep-second".into(),
                    "Or edit by hand:".into(),
                    format!("  1. /{review}"),
                    "  2. edit only the candidate path it returns".into(),
                    "  3. review again, then apply with the new digest".into(),
                ]
                .join("\n"),
            )
        }
        _ => None,
    }
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

fn show(value: &Value) -> Option<String> {
    let job = value.get("job_id")?.as_str()?;
    let children = value.get("children")?.as_array()?;
    let count = match children.len() {
        0 => "no child agents".to_owned(),
        1 => "1 child agent".to_owned(),
        many => format!("{many} child agents"),
    };
    let mut lines = vec![format!(
        "Job {job}: {}, {count}",
        text_of(value.get("state")?)
    )];
    for child in children {
        let mut line = format!(
            "  {}  {}",
            child.get("agent_id")?.as_str()?,
            text_of(child.get("state")?)
        );
        let names = |key: &str| -> Vec<&str> {
            child
                .get(key)
                .and_then(Value::as_array)
                .map(|items| items.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default()
        };
        let paths = names("write_paths");
        if !paths.is_empty() {
            line.push_str(&format!("  owns {}", paths.join(", ")));
        }
        let waiting = names("blocked_by");
        if !waiting.is_empty() {
            line.push_str(&format!("  waiting on {}", waiting.join(", ")));
        }
        lines.push(line);
    }
    if value
        .get("next_cursor")
        .is_some_and(|cursor| !cursor.is_null())
    {
        lines.push("  … more child agents exist".into());
    }
    Some(lines.join("\n"))
}
