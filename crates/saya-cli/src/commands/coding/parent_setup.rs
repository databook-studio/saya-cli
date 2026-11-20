//! The one-time setup of a root run: the provider, the toolset, the hooks over
//! the run's own working checkout, and the executor that puts the hooks before
//! the tools. It runs once per run, however many turns the run takes, so a
//! further turn never re-opens the workspace it already works in.

use super::parent_turn::{RootSession, TurnHistory};
use super::{CodingError, NativeCodingJob};
use crate::agent::hooks::{HookApprovalGate, HookCaller, HookGate, Hooks};
use crate::agent::{provider, tools::coding::RootTools};
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
    ) -> Result<RootSession, CodingError> {
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
        let system_prompt = Some(super::super::prompts::system_with_file_sources(
            self.request.plan,
            self.is_read_only_root(),
            file_sources,
        ));
        self.record_observation_boundary("started", false).await?;
        Ok(RootSession {
            provider,
            tools,
            hooks,
            approval_gate,
            executor,
            checkout,
            system_prompt,
            model: config.ai.model.clone(),
            mailbox: Mutex::new(Some(mailbox)),
            // A run opens with no completed turn: its first turn carries no
            // history, exactly as it always did.
            history: TurnHistory::new(),
        })
    }
}
