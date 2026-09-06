//! Result-set consensus for candidate answers.
//!
//! Pure selection logic: compare what candidate queries *return*, not what
//! their SQL says. Two differently-written queries that produce the same table
//! are the same answer. Nothing here runs SQL, touches a database, or calls a
//! model — a later task wires this into the agent loop.

mod fingerprint;
mod tally;

pub use fingerprint::fingerprint;
pub use tally::{Candidate, Consensus, tally};

#[cfg(test)]
mod tests;
