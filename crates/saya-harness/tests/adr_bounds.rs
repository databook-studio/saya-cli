//! ADR 0009's bounds table against this crate's `pub` constants. Each needle
//! is the ADR's `value | \`CONSTANT\`` cells with the value derived from the
//! constant itself, so a doc number or a code value drifting from the other
//! fails here.

use std::path::PathBuf;

fn adr_text() -> String {
    std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/adr-0009-native-coding-and-continuity.md"),
    )
    .expect("docs/adr-0009-native-coding-and-continuity.md exists at the repo root")
}

fn assert_adr_states(needle: String) {
    let adr = adr_text();
    assert!(
        adr.contains(&needle),
        "the ADR bounds table must state `{needle}`; either the doc number \
         or this crate's constant moved"
    );
}

#[test]
fn adr_states_the_workspace_io_and_patch_bounds() {
    assert_adr_states(format!(
        "{} B | `MAX_IO_BYTES`",
        saya_harness::workspace::MAX_IO_BYTES
    ));
    assert_adr_states(format!(
        "{} B | `PATCH_REPLACEMENT_MAX_BYTES`",
        saya_harness::workspace::patch::PATCH_REPLACEMENT_MAX_BYTES
    ));
}
