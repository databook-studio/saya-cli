//! The permission-mode switch, the keyboard cycle, and the launch posture:
//! the pair a switch sets, the gate that keeps `bypass` unreachable without
//! the configuration's consent, the cycle that skips it, the launch's own
//! reading of a record, and the six values of the one consent axis.

#[cfg(test)]
#[path = "mode_tests/support.rs"]
mod support;

#[cfg(test)]
#[path = "mode_tests/grammar_and_cycle.rs"]
mod grammar_and_cycle;

#[cfg(test)]
#[path = "mode_tests/cycle.rs"]
mod cycle;

#[cfg(test)]
#[path = "mode_tests/switch.rs"]
mod switch;

#[cfg(test)]
#[path = "mode_tests/launch.rs"]
mod launch;

#[cfg(test)]
#[path = "mode_tests/running_job.rs"]
mod running_job;

#[cfg(test)]
#[path = "mode_tests/bypass_gate.rs"]
mod bypass_gate;

#[cfg(test)]
#[path = "mode_tests/round_trip.rs"]
mod round_trip;

#[cfg(test)]
#[path = "mode_tests/one_axis.rs"]
mod one_axis;
