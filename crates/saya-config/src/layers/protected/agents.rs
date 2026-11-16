use crate::{AgentsFile, ConfigFile};

pub(super) fn snapshot(file: &ConfigFile) -> AgentsFile {
    file.agents.clone()
}

pub(super) fn revert(file: &mut ConfigFile, trusted: &AgentsFile) -> bool {
    if file.agents == *trusted {
        false
    } else {
        file.agents = trusted.clone();
        true
    }
}
