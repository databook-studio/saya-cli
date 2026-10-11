//! Source clock for client-side physical-attempt intervals, never remote execution proof.
use crate::{EffectOutcome, EffectOwnerFailure, TokenUsage};
use std::{
    sync::OnceLock,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

struct Clock {
    origin: Instant,
    domain: String,
}
fn clock() -> Result<&'static Clock, EffectOwnerFailure> {
    static CLOCK: OnceLock<Result<Clock, ()>> = OnceLock::new();
    CLOCK
        .get_or_init(|| {
            let wall = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| ())?;
            Ok(Clock {
                origin: Instant::now(),
                domain: format!("p{}-t{}", std::process::id(), wall.as_nanos()),
            })
        })
        .as_ref()
        .map_err(|_| EffectOwnerFailure::new(std::io::Error::other("physical clock unavailable")))
}
pub(super) fn stamp() -> Result<u64, EffectOwnerFailure> {
    u64::try_from(clock()?.origin.elapsed().as_nanos())
        .map_err(|_| EffectOwnerFailure::new(std::io::Error::other("physical clock overflow")))
}

/// Captured before any observer/persistence await. The shared clock domain is
/// comparable only within this client process. A missing start means the send
/// future was never polled; a start does not prove bytes reached a remote host.
#[derive(Debug, Clone)]
pub struct PhysicalAttemptObservation {
    clock_domain: String,
    process_id: u32,
    dispatch_started_ns: Option<u64>,
    finished_ns: u64,
    outcome: EffectOutcome,
    reported_usage: Option<TokenUsage>,
    usage_complete: bool,
}
impl PhysicalAttemptObservation {
    pub(super) fn finish(
        start: Option<u64>,
        outcome: EffectOutcome,
        usage: Option<TokenUsage>,
        complete: bool,
    ) -> Result<Self, EffectOwnerFailure> {
        Ok(Self {
            clock_domain: clock()?.domain.clone(),
            process_id: std::process::id(),
            dispatch_started_ns: start,
            finished_ns: stamp()?,
            outcome,
            reported_usage: usage,
            usage_complete: complete,
        })
    }
    pub fn clock_domain(&self) -> &str {
        &self.clock_domain
    }
    pub fn process_id(&self) -> u32 {
        self.process_id
    }
    pub fn dispatch_started_ns(&self) -> Option<u64> {
        self.dispatch_started_ns
    }
    pub fn finished_ns(&self) -> u64 {
        self.finished_ns
    }
    pub fn outcome(&self) -> EffectOutcome {
        self.outcome
    }
    pub fn reported_usage(&self) -> Option<TokenUsage> {
        self.reported_usage
    }
    pub fn usage_complete(&self) -> bool {
        self.usage_complete
    }
}
