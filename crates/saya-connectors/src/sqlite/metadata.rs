//! SQLite schema discovery: the discovery rounds, the table and nullability
//! construction, and the single-statement foreign-key load.

mod discover;
mod foreign_keys;
mod table;

pub(crate) use discover::schema;
