//! Unit tests for [`redact_param_values`]: every persisted/echoed copy of a
//! submitted command line shows `--param name=…` (D2), while lines that are
//! not investigation commands pass through unchanged.

use super::redact_param_values as redact;

#[path = "param_redact_tests/basic_cases.rs"]
mod basic_cases;
#[path = "param_redact_tests/edge_regressions.rs"]
mod edge_regressions;
