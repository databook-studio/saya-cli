mod calendar;
mod data;
#[cfg(test)]
mod demo_tests;
mod fixture;
mod launch;
mod populate;
mod rng;

use crate::{cli::Cli, commands::result, render::RenderFormat};
use std::{io::IsTerminal, path::Path};

pub(crate) fn run(cli: &Cli, reset: bool) -> Result<i32, Box<dyn std::error::Error>> {
    let dir = fixture::demo_dir();
    let outcome = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(fixture::ensure(&dir, reset))?;
    let db = fixture::db_path(&dir);
    let connections = fixture::connections_path(&dir);
    let can_prompt = !cli.options.non_interactive && std::io::stdin().is_terminal();
    if !can_prompt {
        let format: RenderFormat = cli.options.format.into();
        return result(headless_message(&outcome, &db, &connections), format);
    }
    launch::launch(cli, &connections)
}

fn headless_message(outcome: &fixture::FixtureOutcome, db: &Path, connections: &Path) -> String {
    let state = match outcome {
        fixture::FixtureOutcome::Reused => "reused the existing fixture",
        fixture::FixtureOutcome::Built => "built a new fixture",
    };
    let db = db.display();
    let connections = connections.display();
    format!(
        "Demo database ({state}): {db}\n\
         Connections file: {connections}\n\
         Open it read-only:\n  \
         saya --connections {connections} --profile demo\n\
         Example SQL:\n  \
         SELECT count(*) FROM customers;\n  \
         SELECT count(*) FROM orders WHERE order_date >= '2025-12-31';\n  \
         SELECT c.id, count(*) AS contact_rows FROM customers c \
         JOIN customer_contacts cc ON cc.customer_id = c.id \
         GROUP BY c.id ORDER BY contact_rows DESC LIMIT 5;\n"
    )
}
