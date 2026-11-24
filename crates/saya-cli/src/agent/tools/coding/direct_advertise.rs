//! What the working-checkout root advertises: the one predicate the
//! advertisement and the dispatch gate both read, the evidence it is filtered
//! by, and the causes that grow the list while the run goes.
//!
//! Split from `direct_tools.rs` so the composed toolset and its advertisement
//! stay two concerns: the composition says what exists, this says what the
//! model is offered right now — never a second list, one predicate applied to
//! the composed definitions.

use std::sync::PoisonError;
use std::sync::atomic::Ordering;

use saya_agent::ToolDefinition;

use super::direct_tools::DirectCodingTools;
use crate::agent::tools::shell_policy;

impl DirectCodingTools {
    /// Whether this run has reached `name`: everything the composition
    /// advertised, except the notebook edit where the workspace holds no
    /// notebook, the two background readers before the run that started a
    /// background shell, and — under a closed plan gate — everything the plan
    /// posture does not reach. The one predicate the advertisement and the
    /// dispatch gate both read, so the two cannot disagree about what the model
    /// was offered.
    pub(super) fn advertises(&self, name: &str) -> bool {
        let evidence = self.evidence();
        self.defs.iter().any(|definition| {
            definition.name == name
                && (name != "notebook_edit" || self.advertises_notebook())
                && (!shell_policy::reaches_background(name)
                    || self.background_started.load(Ordering::Acquire))
                && self.plan_permits(definition)
                && self.mode_permits(definition)
                && crate::agent::tools::advertisement::advertised(&evidence, &self.tools, name)
        })
    }

    /// The evidence the optional tools are advertised from, as the run holds it
    /// now: the text it was asked, and whether a web backend is configured.
    pub(super) fn evidence(&self) -> crate::agent::tools::advertisement::Evidence {
        self.evidence
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// States the text this run was asked, once: the first turn's own words.
    ///
    /// A job root is opened with its task, so its first turn's text is the task
    /// the composition already stated — nothing moves. A chat session's request
    /// names the session and never the operator's message, so this is where the
    /// message the run actually answers takes that place. Latching it keeps the
    /// advertisement — and so the request prefix a provider caches — still for
    /// the whole run, exactly as one stated at composition was.
    pub(super) fn note_request_text(&self, text: &str) {
        // The first turn states it, and no later one restates it: the
        // advertisement, and so the request prefix, is the run's.
        if self.evidence_stated.swap(true, Ordering::AcqRel) {
            return;
        }
        let mut evidence = self.evidence.lock().unwrap_or_else(PoisonError::into_inner);
        *evidence =
            crate::agent::tools::advertisement::Evidence::new(text, evidence.web_configured());
    }

    /// Whether the notebook edit is advertised: the workspace holds a
    /// notebook. Decided by the bounded presence check — the same contained,
    /// gitignore-aware walk the searches use, stopped at the first `.ipynb`
    /// name — at the start of each turn until it answers yes, and latched from
    /// then on, so the list grows once for this cause and is stable
    /// afterwards. A workspace the check could not finish offers no notebook
    /// edit: the tool appears only on a definite yes.
    pub(super) fn advertises_notebook(&self) -> bool {
        if self.notebook_seen.load(Ordering::Acquire) {
            return true;
        }
        if crate::agent::tools::workspace::notebook::present(self.workspace.workspace()) {
            self.notebook_seen.store(true, Ordering::Release);
            return true;
        }
        false
    }

    /// Whether the job's plan posture reaches `definition`. With no gate —
    /// every job that never stated `--plan` — the whole composition is reached,
    /// so nothing here filters. With a closed gate, only the read-shaped part
    /// is: the session's own plan rule (`read_only_permits`), which is what
    /// admits `read`, `list`, `glob`, `grep`, and the two session-local tools
    /// `checklist` and `submit_plan`, and refuses the write family, `delete`,
    /// the shell family, `helpers` and `web_fetch`. The operator's approval
    /// opens the rest, and the gate never closes again.
    fn plan_permits(&self, definition: &ToolDefinition) -> bool {
        match &self.plan {
            None => true,
            Some(plan) => plan.is_open() || saya_agent::read_only_permits(&definition.effect),
        }
    }

    /// The advertisement itself: the composed definitions this run has
    /// reached, in composition order.
    pub(super) fn advertised_definitions(&self) -> Vec<ToolDefinition> {
        self.defs
            .iter()
            .filter(|definition| self.advertises(&definition.name))
            .cloned()
            .collect()
    }
}
