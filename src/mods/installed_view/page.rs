//! What the rows of installed mods are called: the exact package's display name and the labels
//! that tell same-named mods apart. Mods › Installed is gone; the mod pages, the Remove review
//! and Home's cards read these.

use super::*;

impl Launcher {
    pub(in crate::mods) fn archive_display(&self, row: &Row) -> Option<&ArchiveDisplay> {
        self.mods_state
            .snapshot
            .as_ref()?
            .archives
            .get(row.archive_file()?)
    }

    pub(in crate::mods) fn labeled_installed_rows(&self) -> (Vec<Row>, Vec<Option<String>>) {
        let mut rows = self.rows();
        // Use the exact installed package's display name here, as in the Remove review.
        // Compute collisions before filtering so a row keeps its label when peers are hidden.
        for row in &mut rows {
            if let Some(archive) = self.archive_display(row) {
                row.title.clone_from(&archive.name);
                if row.authors.is_empty() {
                    row.authors = archive.authors.join(", ");
                }
            }
            if let Some(origin) = nightly::installed_source(row) {
                let update = self.source_update_version(origin).map(str::to_owned);
                if let Some(source) = self.source_listing(origin) {
                    row.authors = source.owner().to_owned();
                    row.collection = self.source_collection_for(source).map(Into::into);
                    row.source_listing = Some(source.clone());
                }
                row.update = update;
            }
            if let Some(origin) = row.local.as_ref().and_then(|local| local.github.as_ref()) {
                if row.authors.is_empty() && !origin.owner.is_empty() {
                    row.authors.clone_from(&origin.owner);
                }
            }
        }
        model::group_installed(&mut rows);
        use installed::{ModLabel, ModLabelSource};
        let labels = installed::colliding_mod_labels(rows.iter().map(|row| {
            let (authors, version) = self.archive_display(row).map_or_else(
                || (std::slice::from_ref(&row.authors), row.version.as_str()),
                |archive| (archive.authors.as_slice(), archive.version.as_str()),
            );
            let source = match &row.local {
                None => ModLabelSource::Registry,
                Some(local) if local.github.is_some() => ModLabelSource::Github,
                Some(_) => ModLabelSource::Local,
            };
            ModLabel {
                id: &row.id,
                name: &row.title,
                authors,
                version,
                source,
            }
        }));
        (rows, labels)
    }
}
