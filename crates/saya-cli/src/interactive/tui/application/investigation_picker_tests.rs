//! Tests for the saved-investigation picker's behaviour: the pure filter,
//! the bounded paginated load, the key actions (Enter shows, `r` replays in
//! the background, Esc closes), and the empty-store state.

use crate::interactive::tui::types::{InvestigationEntry, InvestigationPicker};
use saya_types::SqlDialect;
use saya_types::investigation::{
    INVESTIGATION_FORMAT, INVESTIGATION_FORMAT_VERSION, InvestigationDefinitionV1, InvestigationId,
};

#[path = "investigation_picker_tests/empty_store.rs"]
mod empty_store;
#[path = "investigation_picker_tests/filter.rs"]
mod filter;
#[path = "investigation_picker_tests/key_actions.rs"]
mod key_actions;
#[path = "investigation_picker_tests/load.rs"]
mod load;

fn entry(id: &str, name: &str) -> InvestigationEntry {
    InvestigationEntry {
        id: id.to_owned(),
        name: name.to_owned(),
        label: format!("{:<10}  {name}  ·  sqlite  ·  warehouse", "just now"),
    }
}

fn picker(entries: Vec<InvestigationEntry>) -> InvestigationPicker {
    InvestigationPicker {
        entries,
        selected: 0,
        capped: false,
        query: String::new(),
    }
}

fn definition(id: &str, revision: u32) -> InvestigationDefinitionV1 {
    InvestigationDefinitionV1 {
        format: INVESTIGATION_FORMAT.to_owned(),
        version: INVESTIGATION_FORMAT_VERSION,
        id: InvestigationId::parse(id).unwrap(),
        revision,
        name: "Recent orders".to_owned(),
        description: None,
        sql: format!("select {revision}"),
        parameters: Vec::new(),
        dialect: SqlDialect::Sqlite,
        connection: "warehouse".to_owned(),
        objects: Vec::new(),
        schema_fingerprint: None,
        created_unix_ms: 1_000,
        updated_unix_ms: 1_000,
    }
}

fn three_entries() -> Vec<InvestigationEntry> {
    vec![
        entry("recent-orders-abcdef01", "Recent orders"),
        entry("cost-by-region-deadbeef", "Cost by region"),
        entry("weekend-traffic-0123abcd", "Weekend traffic"),
    ]
}
