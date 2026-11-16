//! Durable locators for container probes whose cleanup may need recovery.

use std::{
    fs,
    io::{Read as _, Write as _},
    path::Path,
};

use super::{RegistryError, private_fs};

const PREFIX: &str = "probe-";
const MAX_TICKET_BYTES: u64 = 64;
const MAX_PENDING_TICKETS: usize = 8;
const MAX_DIRECTORY_ENTRIES: usize = 256;

pub(super) fn begin(root: &Path, ticket: &str) -> Result<(), RegistryError> {
    if !valid_ticket(ticket) {
        return Err(RegistryError::Conflict);
    }
    let path = root.join(format!("{PREFIX}{ticket}"));
    let mut file = private_fs::create_private_file(&path).map_err(|_| RegistryError::Conflict)?;
    if file
        .write_all(ticket.as_bytes())
        .and_then(|()| file.sync_all())
        .is_err()
    {
        let _ = fs::remove_file(path);
        return Err(RegistryError::Io);
    }
    private_fs::sync_directory(root).map_err(|_| RegistryError::Io)
}

pub(super) fn finish(root: &Path, ticket: &str) -> Result<(), RegistryError> {
    if !valid_ticket(ticket) {
        return Err(RegistryError::Conflict);
    }
    let path = root.join(format!("{PREFIX}{ticket}"));
    match fs::remove_file(path) {
        Ok(()) => private_fs::sync_directory(root).map_err(|_| RegistryError::Io),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Err(RegistryError::Conflict),
        Err(_) => Err(RegistryError::Io),
    }
}

pub(super) fn pending(root: &Path) -> Result<Vec<String>, RegistryError> {
    let mut tickets = Vec::new();
    let mut entries = 0;
    for entry in fs::read_dir(root).map_err(|_| RegistryError::Io)? {
        entries += 1;
        if entries > MAX_DIRECTORY_ENTRIES {
            return Err(RegistryError::Conflict);
        }
        let entry = entry.map_err(|_| RegistryError::Io)?;
        let name = entry.file_name();
        let Some(ticket) = name.to_str().and_then(|name| name.strip_prefix(PREFIX)) else {
            continue;
        };
        if !valid_ticket(ticket) {
            return Err(RegistryError::Conflict);
        }
        if tickets.len() == MAX_PENDING_TICKETS {
            return Err(RegistryError::Conflict);
        }
        let file = private_fs::open_read(&entry.path()).map_err(|_| RegistryError::Conflict)?;
        let mut recorded = String::new();
        file.take(MAX_TICKET_BYTES + 1)
            .read_to_string(&mut recorded)
            .map_err(|_| RegistryError::Conflict)?;
        if recorded != ticket || recorded.len() as u64 > MAX_TICKET_BYTES {
            return Err(RegistryError::Conflict);
        }
        tickets.push(ticket.to_owned());
    }
    Ok(tickets)
}

fn valid_ticket(ticket: &str) -> bool {
    ticket.len() == 48 && ticket.bytes().all(|byte| byte.is_ascii_hexdigit())
}
