//! The snapshot's stored preview and its annotation: the JSON copy of the
//! staged preview written into the metadata table when the snapshot is
//! placed, and the read-back a reused session is built from — so the session
//! comes from the snapshot's own stored state, never from a fresh receipt.

use std::path::Path;

use duckdb::{Connection, Transaction, params};
use saya_harness::file_source::{Preview, PreviewColumn, RESERVED_METADATA_TABLE};
use serde::{Deserialize, Serialize};

use super::{
    contract::{inferred_from_label, inferred_label},
    snapshot::staged_config,
};

/// The preview as stored inside the snapshot's metadata table.
#[derive(Serialize, Deserialize)]
struct StoredPreview {
    delimiter: u8,
    header: bool,
    columns: Vec<StoredColumn>,
    sample_rows: Vec<Vec<String>>,
}

#[derive(Serialize, Deserialize)]
struct StoredColumn {
    name: String,
    null_count: usize,
    inferred: String,
}

impl StoredPreview {
    fn from_preview(preview: &Preview) -> Self {
        Self {
            delimiter: preview.delimiter,
            header: preview.header,
            columns: preview
                .columns
                .iter()
                .map(|column| StoredColumn {
                    name: column.name.clone(),
                    null_count: column.null_count,
                    inferred: inferred_label(column.inferred).to_owned(),
                })
                .collect(),
            sample_rows: preview.sample_rows.clone(),
        }
    }

    fn restore(&self) -> Option<Preview> {
        let columns = self
            .columns
            .iter()
            .map(|column| {
                Some(PreviewColumn {
                    name: column.name.clone(),
                    null_count: column.null_count,
                    inferred: inferred_from_label(&column.inferred)?,
                })
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Preview {
            delimiter: self.delimiter,
            header: self.header,
            columns,
            sample_rows: self.sample_rows.clone(),
        })
    }
}

/// Restores the preview stored in a snapshot's metadata JSON. Any malformed
/// entry is `None`: the caller refuses the reuse.
pub(super) fn preview_from_json(json: &str) -> Option<Preview> {
    serde_json::from_str::<StoredPreview>(json).ok()?.restore()
}

/// Records the contract and the preview in the snapshot's metadata table so
/// a reused session is built from the snapshot's own stored state. Runs on
/// the staging database before it is placed, transactionally; any failure
/// fails the staging (the guard removes the staging directory).
pub(super) fn annotate(db_path: &Path, contract: &str, preview: &Preview) -> Result<(), String> {
    let connection = Connection::open_with_flags(db_path, staged_config(false)?).map_err(|_| {
        format!(
            "could not open {} to record its parse contract",
            db_path.display()
        )
    })?;
    let json = serde_json::to_string(&StoredPreview::from_preview(preview))
        .map_err(|_| "could not serialize the snapshot's stored preview".to_owned())?;
    let tx = Transaction::new_unchecked(&connection)
        .map_err(|_| "could not start the snapshot's contract annotation".to_owned())?;
    let written = (|| {
        for (key, value) in [("contract", contract), ("preview", json.as_str())] {
            tx.execute(
                &format!("INSERT INTO {RESERVED_METADATA_TABLE} VALUES (?, ?)"),
                params![key, value],
            )
            .map_err(|_| format!("could not record the snapshot's {key}"))?;
        }
        Ok(())
    })();
    match written {
        Ok(()) => tx
            .commit()
            .map_err(|_| "could not commit the snapshot's contract annotation".to_owned()),
        Err(error) => {
            let _ = tx.rollback();
            Err(error)
        }
    }
}
