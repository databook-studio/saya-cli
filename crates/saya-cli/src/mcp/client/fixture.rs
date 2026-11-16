//! The stdio MCP server the client's tests run.
//!
//! It is an example target (`examples/mcp_fixture_server.rs`), so every
//! `cargo test`/`cargo nextest` build produces it beside the test binary and
//! the tests start it as a real child process on every platform. A shell
//! script could not be started on Windows, and a mock would replace exactly
//! the transport these tests pin.
//!
//! The mode is read from `FIXTURE_MODE`, so a test states the misbehaviour in
//! the server's own configuration (`env`) rather than in the fixture's code.

use std::path::PathBuf;

/// The fixture server built beside this test binary.
///
/// The artifact layout differs between `cargo test` and a custom target
/// directory, so the lookup walks up from the running test binary until it
/// finds the `examples` directory the build put beside it.
pub(crate) fn server_binary(label: &str) -> PathBuf {
    let name = if cfg!(windows) {
        "mcp_fixture_server.exe"
    } else {
        "mcp_fixture_server"
    };
    let exe = std::env::current_exe().expect("the test binary path is known");
    for ancestor in exe.ancestors() {
        let candidate = ancestor.join("examples").join(name);
        if candidate.is_file() {
            return candidate;
        }
    }
    panic!(
        "the MCP fixture server example must be built for {label}; looked beside {}",
        exe.display()
    );
}
