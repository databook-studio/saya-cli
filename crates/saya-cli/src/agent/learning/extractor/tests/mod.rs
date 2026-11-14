use super::*;
use saya_types::{ClaimPayload, ColumnRole};

fn setup_test_table() -> TurnObjectTable {
    let mut table = TurnObjectTable::new();
    table.register(
        "primary",
        "catalog.public.orders",
        &["id".into(), "created_at".into(), "shipped_at".into()],
    );
    table.register(
        "primary",
        "catalog.public.users",
        &["user_id".into(), "email".into()],
    );
    table
}
use super::fences::strip_markdown_fences;
use crate::agent::learning::{ProposalOrigin, TurnObjectId};
use saya_types::KnowledgeSlot;

mod fences;
mod parsing;
mod structured;
