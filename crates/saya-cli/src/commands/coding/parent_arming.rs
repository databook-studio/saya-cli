//! A core turn's plugin budget, armed the first time the turn uses a plugin.
//!
//! A plugin states the policy it wants a turn bounded by only once the turn
//! actually uses it (`RunPlugin::policy_on_use`): a turn that never touches the
//! plugin must not spend a request, tool call, known token or second of it, and
//! the database budget arms on a turn's *first database use*, not when the turn
//! starts (`ref/05`, §2-§3). This module is that decision for one core turn.
//!
//! Two things arm it, and nothing else does:
//!
//! * the plugin's `context()` contributed a block to this turn's prompt, before
//!   request 1 — plugin context reached the model, so the turn used it;
//! * a tool call routed to the plugin (`parent_plugins::Routed`), before it
//!   executes — the turn spent the plugin's tool.
//!
//! The routing decision is the model's, never `request.task`: a prepared run's
//! label arms nothing (`ref/SPIKE-S0`, "Implications — C43"). The readiness
//! precompute never reaches this type, so it never arms a budget either.
//!
//! The budget itself is one [`DeferredAnsweringScope`] on the turn's own token,
//! held unarmed by the loop for the whole turn: unarmed it counts and refuses
//! nothing, and a stop it latches ends the turn and never the run.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use crate::budget_wording::budget_dimension;
use saya_agent::{
    ActivePlugins, BudgetDimension, CancellationReason, CancellationToken, ContextBlock,
    DeferredAnsweringScope, NoopEventSink, TurnPolicy,
};
use tokio::sync::Notify;

/// One core turn's plugin budget, as the core holds it while the turn runs.
#[derive(Clone)]
pub(super) struct TurnArming {
    /// The turn's deferred budget: created unarmed on the turn's own token.
    scope: DeferredAnsweringScope,
    /// The plugins this turn may use, or `None` for a run that consults no
    /// registry — every job surface that supplies none.
    plugins: Option<Arc<ActivePlugins>>,
    /// The turn's own stop signal, the one token a budget stop latches.
    turn: CancellationToken,
    /// Signalled on the first arm, so the elapsed `select!` starts its sleep at
    /// the arm and never before it.
    armed: Arc<Notify>,
    /// Read without the scope's lock, so the route wrapper can tell whether it
    /// is the call that armed the turn.
    is_armed: Arc<AtomicBool>,
}

impl TurnArming {
    /// Holds one turn's plugin budget, unarmed, on the turn's own token.
    pub(super) fn new(plugins: Option<Arc<ActivePlugins>>, turn: CancellationToken) -> Self {
        Self {
            scope: DeferredAnsweringScope::new(turn.clone()),
            plugins,
            turn,
            armed: Arc::new(Notify::new()),
            is_armed: Arc::new(AtomicBool::new(false)),
        }
    }

    /// The scope the turn's loop counts requests, tool batches and reported
    /// tokens through.
    pub(super) fn scope(&self) -> DeferredAnsweringScope {
        self.scope.clone()
    }

    /// Arms the budget when a plugin put context in this turn's prompt: that
    /// context reached the model, so the turn used the plugin. Called before
    /// request 1, which is therefore the turn's first budgeted request.
    pub(super) async fn arm_on_context(&self, blocks: &[ContextBlock]) {
        if !blocks.is_empty() {
            self.arm().await;
        }
    }

    /// Arms the budget when a tool call routes to a plugin, before the call
    /// executes, and charges the turn the request whose answer asked for it:
    /// an armed turn counts what it has already spent, so the policy bounds the
    /// turn rather than handing a fresh allowance to the request after the use.
    /// Only the call that arms the turn charges; a later routed call does not.
    pub(super) async fn arm_on_route(&self) {
        if self.arm().await {
            // Bookkeeping, not a served request: no observation is reported for
            // it, and an unarmed scope would not have counted it either.
            let _ = self.scope.admit_request(&NoopEventSink).await;
        }
    }

    /// The policy this run's plugins state for use in a turn, if any.
    fn policy(&self) -> Option<TurnPolicy> {
        self.plugins
            .as_ref()
            .and_then(|plugins| plugins.policy_on_use())
    }

    /// Arms the turn's budget once. Returns whether this call was the one that
    /// armed it: the first use arms, every later use finds it armed.
    async fn arm(&self) -> bool {
        let Some(policy) = self.policy() else {
            return false;
        };
        if self.is_armed.swap(true, Ordering::AcqRel) {
            return false;
        }
        self.scope.arm(policy).await;
        // The elapsed owner wakes here, so its deadline is the arm's + the
        // policy's, never the turn's start + it.
        self.armed.notify_one();
        true
    }

    /// The budget dimension that stopped this turn, or `None` for a turn a
    /// plugin's policy did not stop. Only an armed turn reads its token: an
    /// unarmed turn has no plugin budget to stop on.
    pub(super) fn stop_dimension(&self) -> Option<BudgetDimension> {
        if self.is_armed.load(Ordering::Acquire) {
            budget_dimension(self.turn.reason())
        } else {
            None
        }
    }

    /// Resolves when the armed elapsed ceiling passes, or never for a turn whose
    /// policy states no elapsed ceiling: a turn that uses no plugin has no
    /// elapsed budget from one, and neither has a turn whose plugin states none.
    /// The sleep is the scope's own remaining deadline on the tokio clock, so the
    /// wait agrees with every other reading of that clock.
    pub(super) async fn expire(&self) {
        self.armed.notified().await;
        loop {
            let Some(remaining) = self.scope.remaining_elapsed().await else {
                // Armed without an elapsed ceiling: only a tighter arm could
                // state one, so the wait continues at the next arm.
                self.armed.notified().await;
                continue;
            };
            if remaining.is_zero() {
                return;
            }
            tokio::time::sleep(remaining).await;
        }
    }
}

/// Latches an elapsed-budget stop on the turn's own token, the one owner a
/// budget stop has.
pub(super) fn stop_the_turn(turn: &CancellationToken) {
    turn.cancel_with_reason(CancellationReason::Budget(BudgetDimension::ElapsedTime));
}

/// Sleeps the elapsed deadline a turn's explicit policy states, or never when
/// it states none — the owner that dimension already had (3.4b).
pub(super) async fn stated_delay(deadline: Option<Duration>) {
    match deadline {
        Some(delay) => tokio::time::sleep(delay).await,
        None => std::future::pending().await,
    }
}
