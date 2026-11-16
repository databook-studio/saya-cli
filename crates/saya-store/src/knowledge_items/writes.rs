//! Mutations of the `knowledge_items` table: the insert/replace put path,
//! light row amendments, the forget tombstone, and the physical cleanup the
//! erasing paths share.

mod amend;
mod cleanup;
mod forget;
mod insert;

pub(crate) use amend::{delete_item, revalidate_item, update_state};
pub(crate) use forget::forget_item;
pub(crate) use insert::insert_or_replace;
