use crate::cli::{Cli, GlobalOptions};
use std::path::Path;

pub(crate) fn launch(cli: &Cli, connections: &Path) -> Result<i32, Box<dyn std::error::Error>> {
    let session = Cli {
        options: GlobalOptions {
            connections: Some(connections.to_path_buf()),
            profile: Some("demo".to_owned()),
            ..cli.options.clone()
        },
        command: None,
    };
    crate::interactive::run(session)
}
