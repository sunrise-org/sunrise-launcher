//! What a bulk removal names: the labels the Remove review shows.

use super::*;

/// Keep removal reviews on the same labels Installed paints, including hidden dependents.
pub(crate) fn remove_names(rows: &[Row], labels: &[Option<String>]) -> BTreeMap<String, String> {
    rows.iter()
        .zip(labels)
        .filter(|(row, _)| row.selectable_archive())
        .map(|(row, label)| (row.id.clone(), label.as_ref().unwrap_or(&row.title).clone()))
        .collect()
}
