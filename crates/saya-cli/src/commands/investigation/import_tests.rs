//! Tests for `saya investigation import`'s raced create (R4-7): the
//! concurrent import that wins between the pre-check and the create decides
//! the outcome by what is stored THEN — identical content is the idempotent
//! no-op, different content the refusal — never the stale "different
//! content" refusal for identical content.

use super::import_with_racer;
use crate::commands::{capture_output_start, capture_output_take};
use crate::render::RenderFormat;
use saya_store::InvestigationRepository;
use saya_types::investigation::InvestigationDefinitionV1;
use std::{
    fs,
    path::{Path, PathBuf},
};

/// A hand-written, fully valid v1 definition, as an exporter on another
/// machine would have produced (the `tests/investigation_share.rs` fixture).
const DEFINITION_JSON: &str = r#"{
  "format": "saya.investigation",
  "version": 1,
  "id": "order-events-01234567",
  "revision": 1,
  "name": "Order events",
  "sql": "SELECT id, label FROM events ORDER BY id",
  "dialect": "sqlite",
  "connection": "local",
  "objects": ["events"],
  "schema_fingerprint": null,
  "created_unix_ms": 1700000000000,
  "updated_unix_ms": 1700000000000
}"#;

fn temp_root(label: &str) -> PathBuf {
    let root =
        std::env::temp_dir().join(format!("saya-import-raced-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

/// Writes the definition file and parses it — the document the import and
/// its racing competitor both carry.
fn definition_file(root: &Path) -> (PathBuf, InvestigationDefinitionV1) {
    let file = root.join("order.json");
    fs::write(&file, DEFINITION_JSON).unwrap();
    let definition =
        InvestigationDefinitionV1::from_json_bytes(DEFINITION_JSON.as_bytes()).unwrap();
    (file, definition)
}

/// The same id, different content: the rename a competing import carries.
fn renamed_definition() -> InvestigationDefinitionV1 {
    let renamed =
        DEFINITION_JSON.replace("\"name\": \"Order events\"", "\"name\": \"Renamed events\"");
    InvestigationDefinitionV1::from_json_bytes(renamed.as_bytes()).unwrap()
}

/// The raced import: the racing competitor's create runs between the
/// pre-check and ours, so our create loses with Conflict — and identical
/// content is the idempotent no-op, not a "different content" refusal.
#[test]
fn raced_identical_import_is_the_idempotent_no_op() {
    let root = temp_root("identical");
    let repo = InvestigationRepository::new(root.join("investigations"));
    let (file, definition) = definition_file(&root);

    capture_output_start();
    let code = import_with_racer(&repo, RenderFormat::Text, &file, &|| {
        repo.create(&definition)
            .expect("the racing competitor stored the identical document first");
    })
    .unwrap();
    let (out, err) = capture_output_take();

    assert_eq!(code, 0, "the raced identical import is a no-op: {out}{err}");
    assert!(
        out.contains("Already present and identical"),
        "the idempotent outcome is said: {out}"
    );
    assert!(
        repo.get(&definition.id).unwrap() == definition,
        "the stored document is the competitor's identical one"
    );
    let _ = fs::remove_dir_all(root);
}

/// The race is only idempotent for identical content: a competitor that
/// stored different content under the same id still refuses, and its
/// document stands.
#[test]
fn raced_different_content_import_still_refuses() {
    let root = temp_root("different");
    let repo = InvestigationRepository::new(root.join("investigations"));
    let (file, _definition) = definition_file(&root);
    let renamed = renamed_definition();

    capture_output_start();
    let code = import_with_racer(&repo, RenderFormat::Text, &file, &|| {
        repo.create(&renamed)
            .expect("the racing competitor stored different content first");
    })
    .unwrap();
    let (out, err) = capture_output_take();

    assert_eq!(code, 2, "{out}{err}");
    assert!(
        err.contains("already exists with different content"),
        "err: {err}"
    );
    assert!(
        repo.get(&renamed.id).unwrap() == renamed,
        "the refusal changes nothing: the competitor's document stands"
    );
    let _ = fs::remove_dir_all(root);
}
