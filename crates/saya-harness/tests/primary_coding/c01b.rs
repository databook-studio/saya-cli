#[path = "c01b/compatibility.rs"]
mod compatibility;
#[path = "c01b/input.rs"]
mod input;
#[path = "c01b/lifecycle.rs"]
mod lifecycle;
#[path = "c01b/spawned.rs"]
mod spawned;

use std::time::Duration;

use saya_harness::coding::commands::{CommandProcessState, CommandSession};

async fn wait_for_state(session: &CommandSession, expected: CommandProcessState) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if session.process_state() == expected {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("runner publishes the expected process lifecycle state");
}
