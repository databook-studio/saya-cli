use std::{collections::HashMap, path::PathBuf, sync::Mutex};

use saya_harness::workspace::Workspace;
use serde_json::Value;

const MAX_WITNESSES: usize = 4_096;
const MAX_WITNESS_PATH_BYTES: usize = 512;
/// The bound on one path's run-produced digest chain. A model that batches
/// edits can produce a long chain in one turn, so the history is capped
/// rather than unbounded; the oldest produced digest is dropped first, while
/// the chain's origin stays.
const MAX_CHAIN_DIGESTS: usize = 64;

/// The run's read witnesses: one observed digest per path, from the last
/// completed read or landed mutation of that path anywhere in this run. A
/// mutation also keeps the chain of digests this run produced on top of the
/// last state it only observed, so a batched edit's stale digest can be told
/// apart from a foreign change. The table survives the invocation boundary
/// the loop raises at a pause and at the end of the run — an entry is dropped
/// only by a workspace rebind, by a failed post-edit diagnostic, or by the
/// path limit.
#[derive(Default)]
pub(in crate::agent::tools) struct ReadWitnesses {
    entries: Mutex<HashMap<String, Entry>>,
}

#[derive(Default)]
struct Entry {
    generation: u64,
    active_reads: usize,
    overlapping_reads: bool,
    witness: Option<Witness>,
    /// The digest this run's last landed write or edit of this path produced.
    /// Kept apart from `witness` because a read clears the observation while
    /// the read's own result still needs to know whether it is looking at
    /// content this run just produced.
    last_mutation: Option<Mutation>,
}

struct Mutation {
    workspace_root: PathBuf,
    digest: String,
}

struct Witness {
    workspace_root: PathBuf,
    digest: String,
    size: u64,
    /// The digest this run's mutation chain started from — the last state the
    /// run observed but did not produce. `None` before the first mutation of
    /// a path (a read witness) and after a mutation on an absent path.
    chain_origin: Option<String>,
    /// The digests this run produced on top of `chain_origin`, oldest first.
    /// The last one is `digest`. A stale `expected_digest` may be re-anchored
    /// only while the current digest is this run's own latest state.
    produced: Vec<String>,
}

#[derive(Clone)]
pub(in crate::agent::tools) struct ReadTicket {
    path: String,
    generation: u64,
}

impl ReadWitnesses {
    pub(in crate::agent::tools) fn begin_read(&self, path: &str) -> Option<ReadTicket> {
        if path.len() > MAX_WITNESS_PATH_BYTES {
            return None;
        }
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if !entries.contains_key(path) && entries.len() >= MAX_WITNESSES {
            let expired = entries.iter().find_map(|(candidate, entry)| {
                (entry.active_reads == 0).then(|| candidate.clone())
            })?;
            entries.remove(&expired);
        }
        let entry = entries.entry(path.to_owned()).or_default();
        entry.overlapping_reads = entry.active_reads > 0;
        entry.active_reads += 1;
        entry.generation = entry.generation.wrapping_add(1);
        entry.witness = None;
        Some(ReadTicket {
            path: path.to_owned(),
            generation: entry.generation,
        })
    }

    pub(in crate::agent::tools) fn finish_read_worker(&self, ticket: &ReadTicket) {
        if let Some(entry) = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get_mut(&ticket.path)
        {
            entry.active_reads = entry.active_reads.saturating_sub(1);
        }
    }

    pub(in crate::agent::tools) fn observe_read(
        &self,
        ticket: ReadTicket,
        workspace: &Workspace,
        digest: String,
        size: u64,
    ) {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let Some(entry) = entries.get_mut(&ticket.path) else {
            return;
        };
        if entry.generation == ticket.generation
            && entry.active_reads == 0
            && !entry.overlapping_reads
        {
            entry.witness = Some(Witness {
                workspace_root: workspace.root().to_path_buf(),
                digest,
                size,
                chain_origin: None,
                produced: Vec::new(),
            });
        }
    }

    pub(in crate::agent::tools) fn snapshot(
        &self,
        path: &str,
        workspace: &Workspace,
    ) -> Option<(String, u64)> {
        if path.len() > MAX_WITNESS_PATH_BYTES {
            return None;
        }
        let entries = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let witness = entries.get(path)?.witness.as_ref()?;
        (witness.workspace_root == workspace.root()).then(|| (witness.digest.clone(), witness.size))
    }

    /// Records one mutation this run landed: `from_digest` is the content the
    /// mutation actually applied against (the digest re-verified at the
    /// commit), and `digest`/`size` are what it produced. The chain lets a
    /// later stale `expected_digest` be re-anchored onto this run's own
    /// state — see [`Self::accepts_stale_digest`] — while a change by anyone
    /// else moves the current digest outside the chain and still refuses.
    pub(in crate::agent::tools) fn remember_mutation(
        &self,
        path: &str,
        workspace: &Workspace,
        from_digest: Option<&str>,
        digest: String,
        size: u64,
    ) {
        if path.len() > MAX_WITNESS_PATH_BYTES {
            return;
        }
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if !entries.contains_key(path) && entries.len() >= MAX_WITNESSES {
            let expired = entries.iter().find_map(|(candidate, entry)| {
                (entry.active_reads == 0).then(|| candidate.clone())
            });
            if let Some(expired) = expired {
                entries.remove(&expired);
            } else {
                return;
            }
        }
        let entry = entries.entry(path.to_owned()).or_default();
        entry.generation = entry.generation.wrapping_add(1);
        entry.overlapping_reads |= entry.active_reads > 0;
        // A chain continues only when the recorded state is exactly the
        // content this mutation applied against; otherwise the mutation
        // starts a new chain at `from_digest`.
        let (chain_origin, mut produced) = match (entry.witness.take(), from_digest) {
            (Some(previous), Some(from))
                if previous.workspace_root == workspace.root() && previous.digest == from =>
            {
                (
                    previous.chain_origin.or_else(|| Some(from.to_owned())),
                    previous.produced,
                )
            }
            (_, Some(from)) => (Some(from.to_owned()), Vec::new()),
            (_, None) => (None, Vec::new()),
        };
        produced.push(digest.clone());
        if produced.len() > MAX_CHAIN_DIGESTS {
            let excess = produced.len() - MAX_CHAIN_DIGESTS;
            produced.drain(..excess);
        }
        let workspace_root = workspace.root().to_path_buf();
        entry.last_mutation = Some(Mutation {
            workspace_root: workspace_root.clone(),
            digest: digest.clone(),
        });
        entry.witness = Some(Witness {
            workspace_root,
            digest,
            size,
            chain_origin,
            produced,
        });
    }

    /// Records a shell view of `path` — one single-file view, exit 0 — at the
    /// file's state right after the call, so a later edit carrying no digest is
    /// admitted as after a `read`. A view is an observation: no chain.
    pub(in crate::agent::tools) fn remember_view(
        &self,
        path: &str,
        workspace: &Workspace,
        digest: String,
        size: u64,
    ) {
        if path.len() > MAX_WITNESS_PATH_BYTES {
            return;
        }
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if !entries.contains_key(path) && entries.len() >= MAX_WITNESSES {
            let expired = entries.iter().find_map(|(candidate, entry)| {
                (entry.active_reads == 0).then(|| candidate.clone())
            });
            if let Some(expired) = expired {
                entries.remove(&expired);
            } else {
                return;
            }
        }
        let entry = entries.entry(path.to_owned()).or_default();
        entry.generation = entry.generation.wrapping_add(1);
        entry.overlapping_reads |= entry.active_reads > 0;
        entry.witness = Some(Witness {
            workspace_root: workspace.root().to_path_buf(),
            digest,
            size,
            chain_origin: None,
            produced: Vec::new(),
        });
    }

    /// Whether `digest` is exactly the state this run's last write or edit of
    /// `path` produced. A read that matches is looking at content the model
    /// already has; the caller adds a note, never a refusal.
    pub(in crate::agent::tools) fn last_mutation_matches(
        &self,
        path: &str,
        workspace: &Workspace,
        digest: &str,
    ) -> bool {
        if path.len() > MAX_WITNESS_PATH_BYTES {
            return false;
        }
        let entries = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        entries
            .get(path)
            .and_then(|entry| entry.last_mutation.as_ref())
            .is_some_and(|mutation| {
                mutation.workspace_root == workspace.root() && mutation.digest == digest
            })
    }

    /// Whether a stale `expected_digest` is stale only because of this run's
    /// own earlier write or edit of the same path. True when the file's
    /// current digest is the latest state this run produced, and the stale
    /// digest is that chain's origin or one of the digests it produced. A
    /// change by anyone else moves the current digest off the chain, so the
    /// caller still refuses. The caller must additionally require the
    /// anchored `old_text` to match exactly once before applying anything.
    pub(in crate::agent::tools) fn accepts_stale_digest(
        &self,
        path: &str,
        workspace: &Workspace,
        stale: &str,
        current: &str,
    ) -> bool {
        if path.len() > MAX_WITNESS_PATH_BYTES {
            return false;
        }
        let entries = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let Some(witness) = entries.get(path).and_then(|entry| entry.witness.as_ref()) else {
            return false;
        };
        witness.workspace_root == workspace.root()
            && witness.digest == current
            && witness.produced.last().is_some_and(|last| last == current)
            && (witness.chain_origin.as_deref() == Some(stale)
                || witness.produced.iter().any(|produced| produced == stale))
    }

    pub(in crate::agent::tools) fn invalidate(&self, path: &str) {
        if let Some(entry) = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get_mut(path)
        {
            entry.generation = entry.generation.wrapping_add(1);
            entry.overlapping_reads |= entry.active_reads > 0;
            entry.witness = None;
        }
    }

    pub(in crate::agent::tools) fn invalidate_if_stale(
        &self,
        path: &str,
        workspace: &Workspace,
        current_digest: &str,
        current_size: u64,
    ) {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let Some(entry) = entries.get_mut(path) else {
            return;
        };
        let stale = entry.witness.as_ref().is_some_and(|witness| {
            witness.workspace_root == workspace.root()
                && (witness.digest != current_digest || witness.size != current_size)
        });
        if stale {
            entry.generation = entry.generation.wrapping_add(1);
            entry.overlapping_reads |= entry.active_reads > 0;
            entry.witness = None;
        }
    }

    pub(in crate::agent::tools) fn clear(&self) {
        self.entries
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clear();
    }
}

impl super::super::database_tools::DatabaseTools {
    /// Fills in the omitted `expected_digest` (and, for `edit`, the omitted
    /// `expected_size`) from the witness of a completed read of the same path
    /// anywhere earlier in this run. The tool compares that witnessed digest
    /// to the file's current bytes at its own fresh read: unchanged admits
    /// the call, changed refuses as stale with the current digest. An
    /// explicit non-null digest always wins — the witness never overrides a
    /// precondition the model stated.
    pub(in crate::agent::tools) fn with_read_witness(
        &self,
        arguments: &Value,
        include_size: bool,
    ) -> Value {
        let Some(path) = arguments.get("path").and_then(Value::as_str) else {
            return arguments.clone();
        };
        let Some(workspace) = self.workspace.as_ref() else {
            return arguments.clone();
        };
        let Some((digest, size)) = self.read_witnesses.snapshot(path, workspace) else {
            return arguments.clone();
        };
        let Some(object) = arguments.as_object() else {
            return arguments.clone();
        };
        if object
            .get("expected_digest")
            .is_some_and(|digest| !digest.is_null())
        {
            return arguments.clone();
        }
        let mut effective = arguments.clone();
        let Some(object) = effective.as_object_mut() else {
            return arguments.clone();
        };
        object.insert("expected_digest".into(), Value::String(digest));
        if include_size && object.get("expected_size").is_none_or(Value::is_null) {
            object.insert("expected_size".into(), Value::from(size));
        }
        effective
    }
}

#[cfg(test)]
#[path = "witness_tests.rs"]
mod tests;
