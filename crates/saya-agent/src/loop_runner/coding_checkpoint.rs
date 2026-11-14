//! Bounded observations of actual tools; context only, never authority.
mod deletion;
mod extract;
mod identity;
mod render;

use saya_types::{AgentId, InvocationId, ManagedCheckoutId, Sha256Digest, WorkspacePath};
use serde::Serialize;

pub(super) const MAX_FACTS: usize = 64;
pub(super) const MAX_FACT_BYTES: usize = 16 * 1024;

/// Trusted identity attached by a child owner, never parsed from a prompt or
/// tool result. It scopes retained authority refusals to one invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorityRefusalContext {
    _invocation_id: InvocationId,
    _authority_epoch: u64,
}

impl AuthorityRefusalContext {
    pub fn new(invocation_id: InvocationId, authority_epoch: u64) -> Self {
        Self {
            _invocation_id: invocation_id,
            _authority_epoch: authority_epoch,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum Fact {
    File {
        path: WorkspacePath,
        digest: Sha256Digest,
        size: u64,
        operation: &'static str,
    },
    Deleted {
        path: WorkspacePath,
        removed_digest: Sha256Digest,
    },
    FollowUp {
        agent_id: AgentId,
        previous_invocation_id: InvocationId,
        invocation_id: InvocationId,
        previous_completion_sequence: u64,
        checkout_id: ManagedCheckoutId,
    },
    AuthorityRefusal {
        tool: String,
        reason_class: &'static str,
        recovery: &'static str,
    },
}

#[derive(Debug, Default)]
pub(super) struct Observation {
    pub(super) facts: Vec<Fact>,
    pub(super) omitted: u64,
    /// Private exact identities are deliberately separate from redacted context.
    pub(super) missing_path: Option<String>,
    pub(super) mutation_path: Option<String>,
    /// Dispatch may have changed source, but produced no confirmed identity.
    pub(super) possible_mutation: bool,
    /// A possible removal invalidates any earlier existence or absence observation.
    pub(super) invalidated_path: Option<String>,
}

impl Observation {
    fn push(&mut self, fact: Fact) {
        if self.facts.len() >= MAX_FACTS {
            self.omitted = self.omitted.saturating_add(1);
            return;
        }
        self.facts.push(fact);
        if serde_json::to_vec(&self.facts).map_or(true, |bytes| bytes.len() > MAX_FACT_BYTES) {
            self.facts.pop();
            self.omitted = self.omitted.saturating_add(1);
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct CodingCheckpoint {
    facts: Vec<Fact>,
    omitted: u64,
    authority: Option<AuthorityRefusalContext>,
}

impl CodingCheckpoint {
    pub(super) fn with_authority(authority: Option<AuthorityRefusalContext>) -> Self {
        Self {
            facts: Vec::new(),
            omitted: 0,
            authority,
        }
    }

    pub(super) fn observe_authority_denial(&mut self, tool: &str, reason_class: &'static str) {
        let Some(_authority) = &self.authority else {
            return;
        };
        if tool.len() > 128 || !tool.is_ascii() {
            self.omitted = self.omitted.saturating_add(1);
            return;
        }
        self.upsert(Fact::AuthorityRefusal {
            tool: tool.to_owned(),
            reason_class,
            recovery: recovery_for(reason_class),
        });
    }
    pub(super) fn observe_serial(&mut self, observation: &Observation) {
        self.omitted = self.omitted.saturating_add(observation.omitted);
        self.invalidate_file(observation);
        for fact in &observation.facts {
            self.upsert(fact.clone());
        }
    }

    /// Collection order of overlapping tools proves no temporal order. Conflicting
    /// same-key observations invalidate the prior fact instead of choosing a winner.
    pub(super) fn observe_batch(&mut self, observations: &[&Observation]) {
        for observation in observations {
            self.omitted = self.omitted.saturating_add(observation.omitted);
            self.invalidate_file(observation);
        }
        let facts = observations
            .iter()
            .flat_map(|observation| &observation.facts)
            .collect::<Vec<_>>();
        let conflict = |fact: &Fact| {
            facts
                .iter()
                .any(|other| fact.same_key(other) && fact != *other)
                || observations.iter().any(|observation| {
                    let path = match fact {
                        Fact::File { path, .. } | Fact::Deleted { path, .. } => path,
                        _ => return false,
                    };
                    observation.invalidated_path.as_deref() == Some(path.as_str())
                        && !observation.facts.contains(fact)
                })
        };
        for fact in &facts {
            if conflict(fact) {
                self.facts.retain(|existing| !existing.same_key(fact));
                self.omitted = self.omitted.saturating_add(1);
            } else {
                self.upsert((*fact).clone());
            }
        }
    }

    fn invalidate_file(&mut self, observation: &Observation) {
        if let Some(path) = &observation.invalidated_path {
            self.facts.retain(|fact| match fact {
                Fact::File { path: existing, .. } | Fact::Deleted { path: existing, .. } => {
                    existing.as_str() != path
                }
                _ => true,
            });
        }
    }

    fn upsert(&mut self, fact: Fact) {
        self.facts.retain(|existing| !existing.same_key(&fact));
        self.facts.push(fact);
        while self.facts.len() > MAX_FACTS
            || serde_json::to_vec(&self.facts).map_or(true, |bytes| bytes.len() > MAX_FACT_BYTES)
        {
            if self.facts.is_empty() {
                break;
            }
            self.facts.remove(0);
            self.omitted = self.omitted.saturating_add(1);
        }
    }
}

fn recovery_for(reason_class: &str) -> &'static str {
    match reason_class {
        "outside_effective_write_paths" => {
            "use_exact_effective_write_paths_or_report_to_coordinator"
        }
        _ => "report_to_coordinator_for_reassignment_or_fresh_authority",
    }
}

pub(super) use extract::observe;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod deletion_tests;
