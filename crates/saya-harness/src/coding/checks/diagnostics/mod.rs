mod location;
mod parse;
mod types;

#[path = "../baseline_wire.rs"]
pub(super) mod baseline_wire;

#[cfg(test)]
mod tests;

pub(super) use parse::parse;
pub use types::{
    Diagnostic, DiagnosticError, DiagnosticLocation, DiagnosticSet, DiagnosticSeverity,
};
