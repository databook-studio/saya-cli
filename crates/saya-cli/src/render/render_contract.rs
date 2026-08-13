//! Text shaping for contract views. JSON and NDJSON fall out of the serde
//! derives on [`TerminalEvent`](super::TerminalEvent); text needs deliberate
//! shaping, done here. See plan spec §3.

use super::{ContractConflictView, ContractView, Rendered};

/// Abbreviation width for display-only claim ids: first six chars + `…` when the
/// id is longer. `ContractChanged.claim_id` is never abbreviated — the user has
/// to be able to paste it into `contracts forget`.
const CLAIM_ID_PREFIX: usize = 6;

fn abbreviate_id(id: &str) -> String {
    if id.len() > CLAIM_ID_PREFIX + 1 {
        format!("{}…", &id[..CLAIM_ID_PREFIX])
    } else {
        id.to_string()
    }
}

pub(super) fn list(contracts: &[ContractView]) -> Rendered {
    if contracts.is_empty() {
        // Not an error: an empty recall simply found nothing. One plain line.
        return Rendered {
            stdout: "No contracts found.\n".into(),
            stderr: String::new(),
        };
    }
    let mut stdout = String::new();
    for contract in contracts {
        stdout.push_str(&stanza(contract));
    }
    Rendered {
        stdout,
        stderr: String::new(),
    }
}

pub(super) fn show(contract: &ContractView) -> Rendered {
    let mut stdout = stanza(contract);
    for conflict in &contract.conflicts {
        stdout.push_str(&conflict_line(conflict));
    }
    if contract.truncated {
        // A truncated contract must never read as a complete one.
        stdout.push_str("  [partial contract — some claims were omitted]\n");
    }
    Rendered {
        stdout,
        stderr: String::new(),
    }
}

pub(super) fn changed(claim_id: &str, action: &str, status: &str) -> Rendered {
    // The full claim id, never abbreviated: the user pastes it into `contracts forget`.
    let line = match action {
        "duplicate" => match status {
            "forgotten" => format!("duplicate of {claim_id} — previously forgotten\n"),
            other => format!("duplicate of {claim_id} — already exists ({other})\n"),
        },
        _ => format!("{action} {claim_id} ({status})\n"),
    };
    Rendered {
        stdout: line,
        stderr: String::new(),
    }
}

/// One contract stanza: a header line followed by one line per claim.
fn stanza(contract: &ContractView) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "{object}  [{state}]{profile}{state_note}\n",
        object = contract.object,
        state = contract.schema_state,
        profile = profile_suffix(&contract.profile),
        state_note = schema_state_note(&contract.schema_state),
    ));
    for claim in &contract.claims {
        out.push_str(&format!(
            "  {id}  {kind}  {status}  {origin}  {value}\n",
            id = abbreviate_id(&claim.claim_id),
            kind = claim.kind,
            status = claim.status,
            origin = claim.origin,
            value = claim.value,
        ));
    }
    out
}

fn profile_suffix(profile: &str) -> String {
    format!("  (profile: {profile})")
}

/// One short clause explaining a non-`current` schema state, appended to the
/// header line so a stale/unreadable schema can't pass as a healthy one.
fn schema_state_note(state: &str) -> &'static str {
    match state {
        "needs_review" => "  — the schema changed since the claim was made",
        "stale" => "  — a column it depends on is gone",
        "live_schema_unavailable" => "  — the schema could not be read",
        _ => "",
    }
}

fn conflict_line(conflict: &ContractConflictView) -> String {
    let claimers = conflict
        .claim_ids
        .iter()
        .map(|id| abbreviate_id(id))
        .collect::<Vec<_>>()
        .join(", ");
    format!("  conflict: {} claimed by {}\n", conflict.kind, claimers)
}
