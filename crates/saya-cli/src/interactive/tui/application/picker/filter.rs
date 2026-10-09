//! Filtering the picker's entries and moving its selection.

use super::super::super::types::{App, Picker, PickerEntry};

impl App {
    /// Entries passing the picker's current filter, in list order.
    pub(crate) fn picker_visible(&self, picker: &Picker) -> Vec<PickerEntry> {
        let needle = picker.query.to_lowercase();
        picker
            .entries
            .iter()
            .filter(|entry| {
                needle.is_empty()
                    || entry.id.to_lowercase().contains(&needle)
                    || entry.label.to_lowercase().contains(&needle)
            })
            .cloned()
            .collect()
    }

    /// Extends the picker's filter; resets the selection to the first match.
    pub(crate) fn picker_char(&mut self, c: char) {
        if let Some(picker) = &mut self.overlays.picker {
            picker.query.push(c);
            picker.selected = 0;
        }
    }

    pub(crate) fn picker_backspace(&mut self) {
        if let Some(picker) = &mut self.overlays.picker {
            picker.query.pop();
            picker.selected = 0;
        }
    }

    /// Moves the picker selection by `delta`, clamped to the filtered list.
    pub(crate) fn picker_move(&mut self, delta: isize) {
        let len = match self.overlays.picker.as_ref() {
            Some(picker) => self.picker_visible(picker).len(),
            None => return,
        };
        if let Some(picker) = &mut self.overlays.picker {
            if len == 0 {
                return;
            }
            let next = (picker.selected as isize + delta).clamp(0, len as isize - 1);
            picker.selected = next as usize;
        }
    }

    /// Confirms the picker selection, requesting a resume in the run loop.
    pub(crate) fn picker_confirm(&mut self) {
        if let Some(picker) = self.overlays.picker.take() {
            let chosen = self
                .picker_visible(&picker)
                .get(picker.selected)
                .map(|entry| entry.id.clone());
            if let Some(id) = chosen {
                self.overlays.pending_resume = Some(id);
            }
        }
    }
}
