use saya_harness::coding::extensions::{
    ExtensionError, ExtensionProcessEvent, ExtensionProcessObserver,
};
use serde_json::json;
use std::sync::{Arc, LazyLock};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

// The production process pool deliberately admits four independent extension
// processes. These eight lifecycle tests all consume that shared real pool,
// so this fixture-only permit prevents unrelated test scheduling from turning
// production-capacity proof into a parallel-test race.
static LIFECYCLE_TEST_SLOT: LazyLock<Arc<Semaphore>> =
    LazyLock::new(|| Arc::new(Semaphore::new(1)));

pub async fn lifecycle_test_slot() -> OwnedSemaphorePermit {
    Arc::clone(&LIFECYCLE_TEST_SLOT)
        .acquire_owned()
        .await
        .expect("fixture semaphore remains owned for the test process")
}

const READ_FRAME: &str = r#"
read_frame() {
    IFS= read -r header || exit 0
    length=${header#Content-Length: }
    # The client's framing::write always emits CRLF, so the header line read
    # by `read` ends with a carriage return; strip it with parameter
    # expansion.
    length=${length%?}
    IFS= read -r blank || exit 0
    # The profile denies process-fork: no external command and no command
    # substitution can run, so the body is consumed with the read builtin.
    # The host /bin/sh is bash 3.2, whose read has no -N; `read -r -n` reads
    # exactly the count when the body holds no newline, which holds because
    # the client writes compact JSON (newlines only as two-byte escapes) and
    # the cleared environment pins the C locale, where -n counts bytes.
    IFS= read -r -n "$length" body || exit 0
}
"#;

fn frames(messages: impl IntoIterator<Item = String>) -> String {
    messages
        .into_iter()
        .map(|message| {
            format!(
                "printf 'Content-Length: {}\\r\\n\\r\\n%s' '{}';",
                message.len(),
                message
            )
        })
        .collect()
}

pub fn after_open(messages: impl IntoIterator<Item = String>) -> String {
    after_open_with_shutdown(messages, response(2))
}

/// A real framed peer whose diagnostic and shutdown responses are independently
/// selectable, for lifecycle precedence tests.
pub fn after_open_with_shutdown(
    messages: impl IntoIterator<Item = String>,
    shutdown: String,
) -> String {
    // `frames` terminates every message with `;` itself, so the block is
    // never followed by another separator: `cmd;;` is a shell syntax error.
    format!(
        "{READ_FRAME} read_frame; {} read_frame; read_frame; {} read_frame; {} read_frame;",
        frames([response(1)]),
        frames(messages),
        frames([shutdown])
    )
}

pub fn after_initialize() -> String {
    format!(
        "{READ_FRAME} read_frame; {} read_frame; read_frame; {} read_frame;",
        frames([response(1)]),
        frames([response(2)])
    )
}

pub fn initialize_with_response(response: String) -> String {
    format!("{READ_FRAME} read_frame; {}", frames([response]))
}

pub fn initialize_then_idle() -> String {
    format!(
        "{READ_FRAME} read_frame; {} read_frame; while IFS= read -r ignored; do :; done",
        frames([response(1)])
    )
}

pub fn response(id: u64) -> String {
    json!({"jsonrpc":"2.0","id":id,"result":null}).to_string()
}

pub fn diagnostics(uri: &str, version: i64, message: &str) -> String {
    json!({"jsonrpc":"2.0","method":"textDocument/publishDiagnostics","params":{
        "uri":uri,"version":version,"diagnostics":[{"range":{"start":{"line":0,"character":0}},"message":message,"severity":1}]
    }})
    .to_string()
}

#[derive(Default)]
pub struct ObservedProcesses(pub std::sync::Mutex<Vec<ExtensionProcessEvent>>);

impl ExtensionProcessObserver for ObservedProcesses {
    fn record(&self, event: ExtensionProcessEvent) -> Result<(), ExtensionError> {
        self.0.lock().unwrap().push(event);
        Ok(())
    }
}

pub fn assert_joined_and_reaped(observer: &ObservedProcesses) {
    let events = observer.0.lock().unwrap();
    let [
        ExtensionProcessEvent::Spawned { pid },
        ExtensionProcessEvent::Settled(receipt),
    ] = events.as_slice()
    else {
        panic!("one actual process must be started and settled: {events:?}")
    };
    assert_eq!(*pid, receipt.pid);
    assert!(receipt.joined);
    assert_eq!(unsafe { libc::kill(*pid as libc::pid_t, 0) }, -1);
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ESRCH)
    );
}
