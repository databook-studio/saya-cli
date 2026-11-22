//! The runtime: one queue in, at most one frame out per change.
//!
//! `04` §10's loop, and nothing else. It blocks on the message queue — there is
//! no polling of its own — folds every message that is already waiting through
//! the pure [`update`](super::update), performs the commands that update
//! answered with, and paints a frame only when the model changed *and* the last
//! frame is at least [`FRAME`] old. An idle surface therefore paints nothing at
//! all, and a burst of a hundred deltas paints at most thirty frames a second,
//! because everything that piled up while one frame was being painted folds into
//! the next one.
//!
//! The terminal is entered and left here, and a panic anywhere inside the loop
//! leaves it the way it was found: the hook restores first and then chains the
//! hook that was installed before it.

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use tokio::sync::mpsc;

use super::model::Model;
use super::msg::Msg;
use super::run_clock::Clock;
use super::run_cmd::{CmdPort, perform};
use super::run_draw::Facts;
use super::run_input;
use super::run_panic::Restored;
use super::run_screen::Screen;
use super::update::update;

/// The shortest gap between two frames: thirty a second, the ceiling the design
/// gives the render loop.
pub(crate) const FRAME: Duration = Duration::from_millis(33);

/// The surface's loop, and everything it owns.
pub(crate) struct Runtime<S: Screen, C: Clock, P: CmdPort> {
    model: Model,
    msgs: mpsc::UnboundedReceiver<Msg>,
    screen: S,
    clock: C,
    port: Arc<P>,
    facts: Facts,
    /// Whether the model has changed since the last frame was painted.
    dirty: bool,
    /// When the last frame was painted.
    painted: Option<Instant>,
    /// When the turn in flight started, for the status line's clock.
    working: Option<Instant>,
    /// Whether something on the surface moves; the ticker reads it.
    animating: Arc<AtomicBool>,
    /// The session's metrics collector, until the first frame is painted: the
    /// launch's own sample, recorded once and taken here.
    metrics: Option<Arc<crate::local_metrics::LocalMetrics>>,
    /// When the launch began, so the first paint's sample measures what the
    /// surface this one replaces measured.
    entered: Instant,
    /// Whether the surface has been asked to leave (`/exit`, `/quit`): the
    /// loop ends once the message that asked has been folded.
    leaving: bool,
}

impl<S: Screen, C: Clock, P: CmdPort> Runtime<S, C, P> {
    /// A loop over one model, one queue, one terminal and one clock.
    pub(crate) fn new(
        model: Model,
        msgs: mpsc::UnboundedReceiver<Msg>,
        screen: S,
        clock: C,
        port: Arc<P>,
    ) -> Self {
        Self {
            model,
            msgs,
            screen,
            clock,
            port,
            facts: Facts::default(),
            dirty: false,
            painted: None,
            working: None,
            animating: Arc::new(AtomicBool::new(false)),
            metrics: None,
            entered: Instant::now(),
            leaving: false,
        }
    }

    /// The collector the first paint's sample is recorded in, and the instant
    /// the launch began — the two facts the launch states and the loop spends
    /// once (`02` §4's first-paint metric).
    pub(crate) fn with_metrics(
        mut self,
        metrics: Option<Arc<crate::local_metrics::LocalMetrics>>,
        entered: Instant,
    ) -> Self {
        self.metrics = metrics;
        self.entered = entered;
        self
    }

    /// The facts every frame is painted with: the launch's token column, glyph
    /// set, provider and workspace. A session that bound no workspace is asked
    /// the trust question here, once, before the loop reads a key (`02` §9.4).
    pub(crate) fn with_facts(mut self, facts: Facts) -> Self {
        if facts.untrusted {
            self.model.open_trust();
        }
        self.facts = facts;
        self
    }

    /// The flag the ticker reads: set while the surface has something that
    /// moves, so an idle surface is never woken.
    pub(crate) fn animating_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.animating)
    }

    /// Runs until the queue closes.
    pub(crate) async fn run(mut self) -> io::Result<()> {
        self.screen.enter()?;
        let restored = Restored::install(self.screen.restorer());
        let outcome = self.parade().await;
        drop(restored);
        let left = self.screen.restore();
        outcome.and(left)
    }

    /// One turn of the loop, with no waiting of its own: fold the message that
    /// is waiting, drain the rest of the burst into the same frame, and paint
    /// the frame that is due. False once the surface has been asked to leave.
    pub(crate) async fn step(&mut self) -> io::Result<bool> {
        let Some(msg) = self.msgs.recv().await else {
            return Ok(false);
        };
        self.fold(msg)?;
        self.drain()?;
        self.paint_if_due()?;
        Ok(!self.leaving)
    }

    /// Blocks on the queue, folding whatever arrives, and paints the frames the
    /// rate limit allows — including the one a burst held back, which is painted
    /// when its 33 ms are up even if nothing else arrives.
    async fn parade(&mut self) -> io::Result<()> {
        loop {
            match self.owed() {
                // A frame is owed and the limit is holding it: wait for the next
                // message or for the frame's turn, whichever comes first.
                Some(owed) => match tokio::time::timeout(owed, self.msgs.recv()).await {
                    Ok(Some(msg)) => self.fold(msg)?,
                    Ok(None) => return Ok(()),
                    Err(_) => {
                        self.paint()?;
                        continue;
                    }
                },
                None => match self.msgs.recv().await {
                    Some(msg) => self.fold(msg)?,
                    None => return Ok(()),
                },
            }
            self.drain()?;
            self.paint_if_due()?;
            if self.leaving {
                return Ok(());
            }
        }
    }

    /// Folds everything already queued into the frame that comes next: a burst
    /// of deltas is one paint, not a hundred.
    fn drain(&mut self) -> io::Result<()> {
        while let Ok(msg) = self.msgs.try_recv() {
            self.fold(msg)?;
        }
        Ok(())
    }

    /// One message: the pure update, its commands performed, and the surface
    /// marked as changed.
    fn fold(&mut self, msg: Msg) -> io::Result<()> {
        // Leaving is the loop's own decision: the message is folded (so the
        // model is exactly as the session left it) and the loop ends after it.
        if matches!(msg, Msg::Quit) {
            self.leaving = true;
        }
        let busy = self.model.is_busy();
        let cmds = update(&mut self.model, msg);
        for cmd in cmds {
            perform(self.port.as_ref(), cmd);
        }
        self.dirty = true;
        match (busy, self.model.is_busy()) {
            (false, true) => self.working = Some(self.clock.now()),
            (true, false) => self.working = None,
            _ => {}
        }
        self.animating
            .store(run_input::animating(&self.model), Ordering::Relaxed);
        Ok(())
    }

    /// How long until a frame may be painted, when one is owed and the limit is
    /// holding it back.
    fn owed(&self) -> Option<Duration> {
        if !self.dirty {
            return None;
        }
        let elapsed = self.since_paint()?;
        (elapsed < FRAME).then(|| FRAME - elapsed)
    }

    /// How long ago the last frame was painted, if one was.
    fn since_paint(&self) -> Option<Duration> {
        let painted = self.painted?;
        Some(self.clock.now().saturating_duration_since(painted))
    }

    /// Paints the frame a change asks for, unless the last one is too recent.
    fn paint_if_due(&mut self) -> io::Result<()> {
        if !self.dirty {
            return Ok(());
        }
        if self.since_paint().is_some_and(|since| since < FRAME) {
            // Owed: the loop's own deadline paints it once its 33 ms are up.
            return Ok(());
        }
        self.paint()
    }

    /// Paints one frame and marks the surface unchanged.
    fn paint(&mut self) -> io::Result<()> {
        let facts = self.facts_now();
        self.screen.draw(&self.model, &facts)?;
        self.dirty = false;
        self.painted = Some(self.clock.now());
        // The first frame's sample: once, from the instant the launch began,
        // into the collector the launch handed over.
        if let Some(metrics) = self.metrics.take() {
            super::run_launch::record_first_paint(&metrics, self.entered);
        }
        Ok(())
    }

    /// The facts this frame is painted with: the launch's, plus how long the
    /// running turn has run.
    fn facts_now(&self) -> Facts {
        Facts {
            working: self
                .working
                .map(|started| self.clock.now().saturating_duration_since(started)),
            ..self.facts.clone()
        }
    }
}
