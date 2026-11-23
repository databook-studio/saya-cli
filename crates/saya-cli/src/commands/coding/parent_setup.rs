//! The one-time setup of a root run: the provider, the toolset, the hooks over
//! the run's own working checkout, and the executor that puts the hooks before
//! the tools. It runs once per run, however many turns the run takes, so a
//! further turn never re-opens the workspace it already works in.

use super::parent_turn::{RootSession, TurnHistory};
use super::parent_turn_limits::TurnInputs;
use super::{CodingError, NativeCodingJob};
use crate::agent::hooks::{HookApprovalGate, HookCaller, HookGate, Hooks};
use crate::agent::tools::child::plugins::active_plugins;
use crate::agent::{provider, tools::coding::RootTools};
use crate::plugins::database::approval::DbApprovalLane;
use saya_agent::{AgentMailbox, ApprovalPolicy, ToolExecutor};
use saya_types::CodingRunFailure;
use std::sync::{Arc, Mutex};

impl NativeCodingJob {
    /// The run's one-time setup. It holds the run's working checkout, so a
    /// turn never re-opens the workspace and a checkpoint is always of the
    /// same directory.
    pub(super) async fn compose_root(
        self: &Arc<Self>,
        mailbox: AgentMailbox,
        inputs: &TurnInputs,
    ) -> Result<RootSession, CodingError> {
        // The plugins this run activates, out of the registry the caller
        // supplied: the same door a helper surface composes them through, so
        // the two cannot turn one registry into two different active sets. A
        // run given no registry activates none and consults none.
        let plugins = inputs
            .plugins
            .as_ref()
            .map(|supplied| Arc::new(active_plugins(&supplied.registry, &supplied.context)));
        // The untrusted project context (instruction sources and the worktree's
        // preferences) rides a chat surface's turns, as the line session carried
        // it on release. A job surface states its own context, so it carries
        // none of it. The same fact decides whether this root states its
        // workspace's session facts: they are a chat session's, not a job's.
        let chat_surface = inputs
            .plugins
            .as_ref()
            .is_some_and(|supplied| supplied.context.surface == saya_agent::RunSurface::Chat);
        let project_context = chat_surface;
        // A direct-mode coordinator owns one working checkout for the whole
        // run; `RootTools::build` ensures it (resume re-ensures it) and composes
        // the toolset. Delegate-only jobs create nothing.
        self.record_provider_binding().map_err(|_| {
            CodingError::Failed(Box::new(CodingRunFailure::root(
                saya_types::CodingFailureClass::MandatoryPersistence,
            )))
        })?;
        let config = &self.application.runtime.resolved;
        self.lifecycle().enter("provider");
        let provider = provider::build(&config.ai, &self.application.runtime.secret_resolver())
            .map_err(|_| {
                CodingError::Failed(Box::new(CodingRunFailure::root(
                    saya_types::CodingFailureClass::ProviderError,
                )))
            })?;
        self.lifecycle().enter("tools");
        let (tools, built_checkout) = RootTools::build(self).await?;
        let mut tools = tools;
        // A chat run's root runs the database plugin on the chat surface, so
        // its database calls are the plugin lane's decision. Every job surface
        // attaches none and keeps the decider it composes: the rule is the
        // lane's own, stated once, so it cannot drift from the plugin's
        // activation.
        if let Some(supplied) = inputs.plugins.as_ref()
            && let Some(lane) =
                DbApprovalLane::for_run(self, &supplied.context, supplied.primary.clone())
        {
            tools.attach_db_lane(Arc::new(lane));
        }
        // A run a session opens works through the session's own task cell: its
        // `checklist` writes there, so the record can keep what the run left,
        // and its turns render the list at the turn's start rather than the
        // root rendering it mid-turn.
        if let Some(tasks) = inputs.tasks.clone() {
            tools.attach_session_tasks(tasks);
        }
        let tools = Arc::new(tools);
        // The run's own working checkout, resolved once: `RootTools::build` has
        // just ensured it and returns the use-time verification it composed the
        // toolset with, so every checkpoint this run takes is of this one
        // directory without a second inventory of it. A read-only surface has
        // no checkout to take; a checkout that cannot be resolved takes no
        // checkpoint, and that refusal is recorded rather than failing the run.
        let checkout = match built_checkout {
            Some(verified) => Some(verified),
            None => match self.verified_root_checkout().await {
                Ok(verified) => verified.map(Arc::new),
                Err(error) => {
                    self.record_capture_refusal(&error);
                    None
                }
            },
        };
        // The user's own hooks, over the run's working checkout — the one
        // directory this run's tools work in. A job whose checkout cannot be
        // resolved runs no hook, the same way it takes no checkpoint. The mode
        // is `bypass`: this root decides its own calls through its own
        // decider, so the host lane a hook runs in is the lane this run
        // already has.
        let hooks = checkout.as_ref().and_then(|verified| {
            Hooks::compose(
                &self.application.runtime,
                ApprovalPolicy::Bypass,
                std::path::Path::new(verified.path().as_str()),
                HookCaller::Job(self.job_id().as_str().to_owned()),
                self.cancellation.clone(),
            )
        });
        // The advisory readiness may be computed while the model streams; the
        // precompute itself decides whether this run's shape allows it.
        self.plan_readiness_precompute(hooks.is_some());
        // The job root's start hook, once; its bounded stdout rides every turn.
        let session_context = match hooks.as_ref() {
            Some(hooks) => hooks.session_start().await,
            None => None,
        };
        let executor: Arc<dyn ToolExecutor> = match hooks.as_ref() {
            Some(hooks) => Arc::new(
                HookGate::new(
                    Arc::clone(hooks),
                    Arc::clone(&tools) as Arc<dyn ToolExecutor>,
                )
                .with_session_context(session_context),
            ),
            None => Arc::clone(&tools) as Arc<dyn ToolExecutor>,
        };
        let approval_gate = hooks.as_ref().map(|hooks| {
            Arc::new(HookApprovalGate::new(
                Arc::clone(hooks),
                Arc::clone(&tools) as Arc<dyn saya_agent::ApprovalDecider>,
            ))
        });
        // The shell route for local data files is decided once, from the
        // checkout this run owns, the `PATH` this process was started with, and
        // whether this root's toolset actually offers `shell`: the route names
        // that tool, so a composition without one must not carry it.
        let file_sources = crate::agent::file_sources::for_session(
            tools.offers_shell(),
            crate::agent::file_sources::detect(
                tools.workspace(),
                std::env::var_os("PATH").as_deref(),
            ),
        );
        let system_prompt = match self.has_workspace() {
            true => super::super::prompts::system_with_file_sources(
                self.request.plan,
                self.is_read_only_root(),
                file_sources,
            ),
            // A run with no workspace reads no file and has no shell, so it is
            // told that rather than told to read a workspace it does not have.
            false => super::super::prompts::NO_WORKSPACE.to_owned(),
        };
        // The active plugins' sections and the session's own facts are stated
        // per turn by `RootSession::prompt_for_turn`, in the order they ride
        // here: a family that latches mid-run has no section at the run's open,
        // so the section a plugin states is composed on the turn that latched
        // it and frozen from then on. The folder fact keeps its place after
        // those sections, exactly where it rode before they were deferred.
        let root_facts = match chat_surface {
            true => checkout.as_ref().map(|verified| {
                crate::agent::session_facts::workspace_root_text(std::path::Path::new(
                    verified.path().as_str(),
                ))
            }),
            false => None,
        };
        let system_prompt = Some(system_prompt);
        self.record_observation_boundary("started", false).await?;
        Ok(RootSession {
            provider,
            tools,
            hooks,
            approval_gate,
            executor,
            checkout,
            system_prompt,
            sections: Mutex::new(None),
            root_facts,
            model: config.ai.model.clone(),
            mailbox: Mutex::new(Some(mailbox)),
            // A run opens with no completed turn, ahead of the conversation it
            // was seeded with: an unseeded run's first turn carries no history,
            // exactly as it always did.
            history: TurnHistory::seeded(inputs.seed.clone()),
            plugins,
            project_context,
            // The session facts a chat run's requests carry, when the session
            // states them: the compaction narrative and the continuation
            // record its loop holds.
            facts: inputs.facts.clone(),
        })
    }
}
