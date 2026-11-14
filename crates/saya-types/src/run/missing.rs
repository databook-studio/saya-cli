//! The scope difference, as words: which scope tokens a capability set asks
//! for that an approval does not grant. The subset *rule* stays
//! [`Capabilities::is_subset_of`] — this module only names what it judged,
//! so a refusal can say what was missing instead of refusing generically.

use super::scope::{Capabilities, EndpointBindings};

impl Capabilities {
    /// Names the scope tokens `self` asks for that `approved` does not
    /// grant — the words the `--allow` grammar speaks (`workspace-write`,
    /// `fetch:<scheme>+<host>`,
    /// `endpoint:<role>=<endpoint>`). Every family is
    /// judged by
    /// [`Capabilities::is_subset_of`] (the one subset rule) on a probe
    /// carrying that family alone; this only names the families and members
    /// that fail it, and is empty exactly when the subset holds.
    pub fn missing_from(&self, approved: &Capabilities) -> Vec<String> {
        let mut missing = Vec::new();
        let mut probe = Capabilities::default();
        if self.workspace_write {
            probe.workspace_write = true;
            if !probe.is_subset_of(approved) {
                missing.push("workspace-write".to_string());
            }
            probe.workspace_write = false;
        }
        if let Some(fetch) = &self.fetch {
            probe.fetch = Some(fetch.clone());
            if !probe.is_subset_of(approved) {
                for destination in &fetch.destinations {
                    let approved_destination = approved
                        .fetch
                        .as_ref()
                        .is_some_and(|theirs| theirs.destinations.contains(destination));
                    if !approved_destination {
                        missing.push(format!("fetch:{}+{}", destination.scheme, destination.host));
                    }
                }
            }
            probe.fetch = None;
        }
        if !self.endpoints.as_map().is_empty() {
            for (role, endpoint) in self.endpoints.as_map() {
                probe.endpoints = EndpointBindings::new([(role.clone(), endpoint.clone())])
                    .expect("a binding already inside a Capabilities keeps its shape");
                if !probe.is_subset_of(approved) {
                    missing.push(format!("endpoint:{role}={endpoint}"));
                }
            }
            probe.endpoints = EndpointBindings::default();
        }
        missing
    }
}
