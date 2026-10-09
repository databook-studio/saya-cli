use super::*;

#[path = "cases/budget.rs"]
mod budget;
#[path = "cases/containment.rs"]
mod containment;
#[path = "cases/create_only.rs"]
mod create_only;
#[path = "cases/destination_replacement.rs"]
mod destination_replacement;
#[path = "cases/refusals.rs"]
mod refusals;
#[path = "cases/resumption.rs"]
mod resumption;
#[path = "cases/tool_gate.rs"]
mod tool_gate;
#[cfg(windows)]
#[path = "cases/windows_resume.rs"]
mod windows_resume;
