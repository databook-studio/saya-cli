//! The advertised tool surface of one provider request: names in wire order,
//! and a short, stable hash of each tool's `parameters` schema.
//!
//! The hash is sha256 of the canonical JSON (serde_json's compact form over a
//! `Value`, whose object keys are sorted) truncated to the first 12 hex
//! characters. Descriptions and tool effects are deliberately excluded: the
//! row asks whether the same *call shape* reaches every surface.

use std::collections::BTreeMap;

use sha2::{Digest, Sha256};

/// One surface's first request: the ordered names and the schema digest each
/// name carried.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Advertised {
    pub(super) names: Vec<String>,
    pub(super) schemas: BTreeMap<String, String>,
}

impl Advertised {
    pub(super) fn from_body(body: &serde_json::Value) -> Self {
        let mut names = Vec::new();
        let mut schemas = BTreeMap::new();
        for tool in body["tools"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default()
        {
            let Some(name) = tool["function"]["name"].as_str() else {
                continue;
            };
            let canonical =
                serde_json::to_vec(&tool["function"]["parameters"]).expect("schema serializes");
            let digest = Sha256::digest(&canonical);
            let short: String = digest
                .iter()
                .take(6)
                .map(|byte| format!("{byte:02x}"))
                .collect();
            names.push(name.to_owned());
            schemas.insert(name.to_owned(), short);
        }
        Self { names, schemas }
    }
}
