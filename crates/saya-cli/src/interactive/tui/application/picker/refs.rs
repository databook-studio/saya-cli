//! `@`-reference names and starter questions reloaded from the cached schema.

use super::super::super::atref;
use super::super::super::types::App;
use super::super::super::ui::surface::starter_questions;
use crate::interactive::session::state::SessionState;
use saya_store::SchemaStore;

impl App {
    /// Reloads `@`-reference names from the cached schema of the active and
    /// included profiles (best-effort; empty when nothing is cached), and —
    /// from the same already-fetched trees, no extra read — the active
    /// profile's starter questions for the empty state's examples. Included
    /// profiles never contribute starter questions: the first screen asks
    /// about the database the session is pointed at, not the included ones.
    pub(crate) fn reload_at_refs(&mut self, state: &SessionState) {
        let mut names: Vec<&str> = Vec::new();
        if let Some(profile) = state.profile.as_deref() {
            names.push(profile);
        }
        names.extend(state.included_profiles.iter().map(String::as_str));
        let mut refs = Vec::new();
        let mut starter = Vec::new();
        for name in names {
            if let Ok(profile) = self.runtime.named_profile(name) {
                let identity = crate::profile_identity::profile_identity(
                    name,
                    profile,
                    &self.runtime.cache_scope,
                );
                if let Ok(Some(cached)) = crate::interactive::session::lifecycle::resume::block_on(
                    self.state_db.get_schema(identity.as_str()),
                ) {
                    refs.extend(atref::schema_refs(&cached.schema));
                    if Some(name) == state.profile.as_deref() {
                        starter = starter_questions(&cached.schema);
                    }
                }
            }
        }
        refs.sort();
        refs.dedup();
        self.at_refs = refs;
        self.starter_questions = starter;
    }
}
