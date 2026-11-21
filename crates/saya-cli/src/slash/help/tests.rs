use super::*;

/// The catalogue's one-line description for a name — the same text the popup
/// and the `/help` listing show, so these tests read one source.
fn description_for(name: &str) -> Option<&'static str> {
    crate::slash::catalog::entries()
        .find(|entry| entry.name == name)
        .map(|entry| entry.description)
}

include!("tests/cases_1.rs");
include!("tests/cases_2.rs");
include!("tests/cases_3.rs");
include!("tests/cases_4.rs");
include!("tests/cases_5.rs");
include!("tests/cases_6.rs");
