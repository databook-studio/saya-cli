//! Shared harness for the deterministic correctness scenarios (B3d).
//!
//! Every scenario builds the real demo fixture with the actual `saya demo`
//! binary (SQLite, fixed seed: customers 240, orders 560, customer_contacts
//! 123), scripts an OpenAI-compatible mock provider round by round, and runs
//! the real `saya ask` binary headlessly. Assertions land on the rows saya
//! returned to the model — read from the tool result the follow-up provider
//! request carries — and on the truth computed independently by a second
//! `saya query` against the same fixture. No network beyond 127.0.0.1; the
//! mock harness is the tests/mvp one, duplicated because tests in different
//! binaries cannot share modules.

#[path = "common/bodies.rs"]
mod bodies;
#[path = "common/capture.rs"]
mod capture;
#[path = "common/fixture.rs"]
mod fixture;
#[path = "common/mock.rs"]
mod mock;

pub(crate) use bodies::{text_body, tool_call_body};
pub(crate) use capture::tool_result_rows;
pub(crate) use fixture::{DemoFixture, build_demo, run_ask, run_query};
pub(crate) use mock::{request_json, spawn_mock};
