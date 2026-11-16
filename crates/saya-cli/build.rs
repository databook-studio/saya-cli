//! Windows records a stack reserve in the executable header, and a thread
//! created with the default size — Rust's main thread and every
//! `std::thread::spawn` without an explicit `stack_size` — takes that
//! reserve. The MSVC toolchain records 1 MiB, which a debug-build run or job
//! future (plan, agent loop, tool dispatch) outgrows: Windows CI saw the
//! child `saya.exe` die with `STATUS_STACK_OVERFLOW` while the same run
//! completed on macOS and Linux. Reserve the headroom those platforms give
//! the main thread so every thread that polls a run or job future inherits
//! it, including workers the engine spawns later.
//!
//! Unix needs no counterpart: its main thread already has 8 MiB.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    // Reserved, not committed: the address space costs nothing until the
    // stack grows into it.
    const STACK_RESERVE_BYTES: usize = 16 * 1024 * 1024;
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg=/STACK:{STACK_RESERVE_BYTES}");
    } else {
        println!("cargo:rustc-link-arg=-Wl,--stack,{STACK_RESERVE_BYTES}");
    }
}
