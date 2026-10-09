#[cfg(unix)]
const HEADER: &str = "saya local metrics (fixed fields; not a benchmark)";

#[cfg(unix)]
const VERBOSE_ARGS: [&str; 1] = ["--verbose"];

#[cfg(unix)]
fn metrics_config() -> String {
    "[ai]\ninvestigation_max_elapsed_seconds = 4\n".to_owned()
}
/// One natural turn, verbose or not: a prompt carrying SENTINEL-PROMPT-1 is
/// answered with SENTINEL-ANSWER-2, the child exits on `/exit`, and the
/// full stdout and stderr buffers come back after the pumps are joined.
#[cfg(unix)]
fn one_turn(tag: &str, verbose: bool) -> (Vec<u8>, Vec<u8>) {
    use std::{
        net::TcpListener,
        sync::{Arc, atomic::AtomicBool},
    };

    use harness::{
        ScriptStep, spawn_child_with_config_extra, spawn_script_server, wait_for_stream,
    };

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let release = Arc::new(AtomicBool::new(false));
    let mut server = spawn_script_server(
        listener,
        vec![ScriptStep::Summary("SENTINEL-ANSWER-2".to_owned())],
        release,
    );

    let extra: &[&str] = if verbose { &VERBOSE_ARGS } else { &[] };
    let mut spawned = spawn_child_with_config_extra(&server.address, &metrics_config(), tag, extra);
    spawned.send(b"SENTINEL-PROMPT-1\n");

    wait_for_stream(&spawned.stdout_buf, b"SENTINEL-ANSWER-2", "answer text");
    spawned.send(b"/exit\n");

    let stdout = spawned.stdout_buf.clone();
    let stderr = spawned.stderr_buf.clone();
    spawned.finish();
    server.join();
    (
        stdout.lock().unwrap().clone(),
        stderr.lock().unwrap().clone(),
    )
}
