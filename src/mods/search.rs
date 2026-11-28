//! What the launcher's search (`command_search`) reads of the installed mods and packs, and how
//! it shows one of them on its page. Reads what is in memory or a few small files; writes
//! nothing.
use super::*;
use std::sync::Arc;
use sunrise_launcher::packs::Kind;

impl Launcher {
    /// The selected profile's installed mods: each one's name and its icon's address.
    pub(crate) fn mods_for_search(&mut self) -> Vec<(String, String, Option<String>)> {
        self.mods_refresh();
        self.rows()
            .into_iter()
            .map(|row| (row.id, row.title, row.icon))
            .collect()
    }

    /// Its resource packs, then its shader packs: each one's kind, name and picture.
    pub(crate) fn packs_for_search(
        &mut self,
    ) -> Vec<(Kind, String, String, Option<Arc<gpui::Image>>)> {
        self.load_packs_list(self.mods_home());
        let packs = &self.game_packs;
        packs
            .resource
            .iter()
            .map(|(pack, icon)| {
                (
                    pack.kind,
                    pack.file.clone(),
                    pack.title.clone(),
                    icon.clone(),
                )
            })
            .chain(
                packs
                    .shader
                    .iter()
                    .map(|pack| (pack.kind, pack.file.clone(), pack.title.clone(), None)),
            )
            .collect()
    }

    /// Opens Packs (the shelves of shader and data packs) at `to` from another page. Whatever the
    /// page was left showing (ticked rows, a question waiting for its answer, filters) is not
    /// carried in: the player sees what they asked for, and nothing they can't see is acted on.
    pub(crate) fn open_installed(&mut self, to: Installed) {
        self.leave_section(true);
        self.ui.page = self.mods_state.enter(&to);
        self.game_packs.confirm_remove = None;
        self.game_packs.stale();
        self.mods_refresh();
    }

    /// The page of the installed mod called `title` (or with that id): its own page, as Home's
    /// card opens it. False when the profile has no such mod.
    pub(crate) fn open_mod_named(&mut self, title: &str) -> bool {
        self.mods_refresh();
        let Some(row) = self
            .rows()
            .into_iter()
            .find(|row| row.title == title || row.id == title)
        else {
            return false;
        };
        self.mods_show_installed_project(&row);
        true
    }
}

/// A place on the Packs shelves that another page opens. The resource packs are listed on
/// Home's Packs tab; the shelves keep the shader and data packs.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Installed {
    /// The shelf of this kind, the packs a search for this name finds.
    PackNamed(Kind, String),
}

impl ModsState {
    /// Starts the shelf afresh at `to`: the page to show.
    pub(super) fn enter(&mut self, to: &Installed) -> Page {
        self.selection.clear();
        self.anchor = None;
        self.menu = None;
        self.unfit_open = None;
        self.have.set(String::new());
        self.row_filter = model::RowFilter::default();
        let Installed::PackNamed(kind, title) = to;
        self.have.set(title.clone());
        self.row_filter.text = title.clone();
        self.shelf = match kind {
            Kind::Resource => Shelf::Resource,
            Kind::Shader => Shelf::Shader,
        };
        self.data.reread();
        Page::Packs
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use model::{Make, RowFilter};

    /// Installed as the player left it: two mods ticked (one of them on), a range begun,
    /// and a filter by kind.
    fn left_behind() -> ModsState {
        let mut m = ModsState {
            selection: ["lantern", "trail"].map(String::from).into(),
            anchor: Some("sunrise-mods/trail.smod".into()),
            row_filter: RowFilter {
                text: "lantern".into(),
                makes: [Make::Content].into(),
            },
            ..ModsState::default()
        };
        m.have.set("lantern".into());
        m
    }

    fn nothing_carried_in(m: &ModsState) {
        assert!(m.selection.is_empty(), "no hidden row stays ticked");
        assert_eq!(m.anchor, None);
        assert!(m.row_filter.makes.is_empty());
    }

    #[test]
    fn home_destinations_open_the_requested_installed_filter_or_shelf() {
        // Wherever Mods and Packs were left: Packs on Discover, its Installed on the data
        // packs' shelf, a name typed that matches nothing.
        let left = || {
            let mut m = left_behind();
            m.shelf = Shelf::Data;
            m.have.set("zz".into());
            m.row_filter.text = "zz".into();
            m
        };
        for (kind, shelf) in [
            (Kind::Resource, Shelf::Resource),
            (Kind::Shader, Shelf::Shader),
        ] {
            let mut m = left();
            assert!(
                m.enter(&Installed::PackNamed(kind, String::new())) == Page::Packs,
                "Installed"
            );
            assert_eq!(m.shelf, shelf, "the requested shelf");
            assert_eq!(shelf.kind(), Some(kind));
            // Every pack of the shelf is listed: no name is searched for.
            assert_eq!((m.have.text(), m.row_filter.text.as_str()), ("", ""));
            nothing_carried_in(&m);
        }
    }
}
