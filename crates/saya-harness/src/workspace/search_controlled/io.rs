use super::types::{Control, SearchError, SearchStop};
use crate::workspace::search_directory::Directory;
use std::io::Read;

pub(super) enum FileRead {
    Missing,
    Bytes(Vec<u8>),
    Unavailable,
}

pub(super) struct ReadBudget {
    pub used: usize,
    pub maximum: usize,
}
impl ReadBudget {
    pub fn read(
        &mut self,
        directory: &Directory,
        path: &str,
        maximum_file: usize,
        control: &Control<'_>,
    ) -> Result<Result<FileRead, SearchStop>, SearchError> {
        if !control.check()? {
            return Ok(Err(SearchStop::Deadline));
        }
        let name = path.rsplit('/').next().unwrap_or(path);
        let (file, declared) = match directory.file(path, name) {
            Ok(Some(file)) => file,
            Ok(None) => return Ok(Ok(FileRead::Missing)),
            Err(_) => return Ok(Ok(FileRead::Unavailable)),
        };
        if declared > maximum_file as u64 {
            return Ok(Ok(FileRead::Unavailable));
        }
        self.read_opened(file, declared, maximum_file, control)
    }
    fn read_opened(
        &mut self,
        mut file: std::fs::File,
        declared: u64,
        maximum_file: usize,
        control: &Control<'_>,
    ) -> Result<Result<FileRead, SearchStop>, SearchError> {
        let mut bytes = Vec::new();
        let mut buffer = [0u8; 64 * 1024];
        loop {
            if !control.check()? {
                return Ok(Err(SearchStop::Deadline));
            }
            if bytes.len() as u64 >= declared {
                match file.metadata() {
                    Ok(metadata)
                        if metadata.len() == declared && bytes.len() as u64 == declared =>
                    {
                        break;
                    }
                    _ => return Ok(Ok(FileRead::Unavailable)),
                }
            }
            if bytes.len() >= maximum_file {
                return Ok(Ok(FileRead::Unavailable));
            }
            let allowance = self
                .maximum
                .saturating_sub(self.used)
                .min(maximum_file - bytes.len())
                .min((declared - bytes.len() as u64) as usize)
                .min(buffer.len());
            if allowance == 0 {
                return Ok(Err(SearchStop::ReadBytes));
            }
            let read = match file.read(&mut buffer[..allowance]) {
                Ok(read) => read,
                Err(_) => return Ok(Ok(FileRead::Unavailable)),
            };
            if read == 0 {
                return Ok(Ok(FileRead::Unavailable));
            }
            self.used = self.used.saturating_add(read);
            if self.used > self.maximum {
                return Ok(Err(SearchStop::ReadBytes));
            }
            if bytes.len().saturating_add(read) > maximum_file {
                return Ok(Ok(FileRead::Unavailable));
            }
            bytes.extend_from_slice(&buffer[..read]);
        }
        if !control.check()? {
            return Ok(Err(SearchStop::Deadline));
        }
        Ok(Ok(FileRead::Bytes(bytes)))
    }
}

#[cfg(test)]
#[path = "io_tests.rs"]
mod tests;
