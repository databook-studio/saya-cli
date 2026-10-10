use saya_types::{MAX_PROJECT_PREFERENCES, ProjectPreference, WorktreeId, validate_set};

use super::limits::ChildContextError;

pub(super) fn validate_preferences(
    preferences: &[ProjectPreference],
    worktree_id: &WorktreeId,
) -> Result<(), ChildContextError> {
    if preferences.len() > MAX_PROJECT_PREFERENCES {
        return Err(ChildContextError::TooManyPreferences);
    }
    for preference in preferences {
        preference
            .validate()
            .map_err(ChildContextError::InvalidPreference)?;
        if &preference.worktree != worktree_id {
            return Err(ChildContextError::ForeignPreference);
        }
    }
    validate_set(preferences).map_err(ChildContextError::InvalidPreferenceSet)
}
