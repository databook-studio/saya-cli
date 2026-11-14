//! The read bound is 256 KiB per call: a file under it is read whole in one
//! call, and a file over it keeps the existing over-bound behaviour — a
//! truncated success at the bound, never a refusal. The model-facing result is
//! still bounded by the loop's message cap, so raising the retention bound
//! does not raise what reaches the model.

use super::*;

/// A 200 KiB file is read whole in one call; a 257 KiB file is truncated at
/// the 256 KiB bound, with its full size and whole-file digest still reported.
#[tokio::test]
async fn the_read_bound_is_256_kib() {
    assert_eq!(WORKSPACE_READ_MAX_BYTES, 256 * 1024);
    let sandbox = Sandbox::new("read-bound");

    let whole = vec![b'a'; 200 * 1024];
    sandbox
        .ws
        .write("whole.bin", &whole)
        .expect("seed write must succeed");
    let file = sandbox
        .ws
        .read("whole.bin", WORKSPACE_READ_MAX_BYTES)
        .expect("a contained read must succeed");
    assert!(!file.truncated, "a file under the bound reads whole");
    assert_eq!(file.bytes.len(), whole.len());

    let over = vec![b'a'; 257 * 1024];
    sandbox
        .ws
        .write("over.bin", &over)
        .expect("seed write must succeed");
    let file = sandbox
        .ws
        .read("over.bin", WORKSPACE_READ_MAX_BYTES)
        .expect("an over-bound read is a truncated success, not a refusal");
    assert!(file.truncated, "a file over the bound is truncated");
    assert_eq!(file.bytes.len() as u64, WORKSPACE_READ_MAX_BYTES);
    assert_eq!(file.size as usize, over.len());
}

/// A 257 KiB read still reaches the model through the existing result cap:
/// the tool bisects its content to the loop's 64 KiB message cap, so the
/// larger retention bound is not a larger model result.
#[tokio::test]
async fn a_read_over_the_bound_still_fits_the_model_message_cap() {
    let sandbox = Sandbox::new("read-model-cap");
    let total = 257 * 1024;
    sandbox
        .ws
        .write("big.bin", &vec![b'a'; total])
        .expect("seed write must succeed");
    let result = sandbox
        .tools()
        .execute("read", serde_json::json!({"path": "big.bin"}))
        .await
        .expect("a bounded read must succeed");
    assert_eq!(result["truncated"], serde_json::Value::Bool(true));
    assert_eq!(result["size"].as_u64(), Some(total as u64));
    let shaped = saya_agent::shape_tool_result(&result, saya_agent::MAX_TOOL_MESSAGE_BYTES);
    assert!(
        !shaped.truncated,
        "the tool's result already fits the model's message cap"
    );
    assert!(shaped.text.len() <= saya_agent::MAX_TOOL_MESSAGE_BYTES);
}
