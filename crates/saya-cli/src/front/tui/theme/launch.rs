//! The token column a launch paints with, from the environment it read once.
//!
//! The rule is no-color.org's: `NO_COLOR` set to *any* non-empty value asks for
//! no colour, and an empty value states nothing. The launch reads the value once
//! and hands the map here, so this resolver stays a pure function — a test
//! states its own environment instead of mutating the process's.

use std::collections::BTreeMap;

use super::tokens::Scheme;

/// The variable the convention names.
pub(crate) const NO_COLOR: &str = "NO_COLOR";

/// The token column a launch paints with: the no-colour column when the
/// environment asks for it, else the palette the launch chose.
pub(crate) fn scheme(env: &BTreeMap<String, String>, chosen: Scheme) -> Scheme {
    if asks_for_no_colour(env) {
        Scheme::NoColour
    } else {
        chosen
    }
}

/// Whether `env` asks for no colour. no-color.org's rule is deliberately
/// literal: any non-empty value, including one that reads like a falsehood,
/// disables colour; an empty value states nothing.
pub(crate) fn asks_for_no_colour(env: &BTreeMap<String, String>) -> bool {
    env.get(NO_COLOR).is_some_and(|value| !value.is_empty())
}
