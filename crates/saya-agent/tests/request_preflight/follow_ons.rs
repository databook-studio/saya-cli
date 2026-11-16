use super::*;

#[derive(Debug, Clone, Copy)]
enum Route {
    Continuation,
    Designation,
    Salvage,
}

#[path = "follow_ons/continuation_admission.rs"]
mod continuation_admission;
#[path = "follow_ons/pre_dispatch_routes.rs"]
mod pre_dispatch_routes;
