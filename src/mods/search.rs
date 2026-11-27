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

    /// Opens Installed at `to` from another page. Whatever the page was left showing (ticked
    /// rows, a question waiting for its answer, filters) is not carried in: the player sees
    /// what they asked for, and nothing they can't see is acted on.
    pub(crate) fn open_installed(&mut self, to: Installed) {
        let before = self.ui.page;
        self.multiplayer.target = None;
        self.leave_section(!matches!(
            to,
            Installed::Mods | Installed::ModsOff | Installed::Mod(_) | Installed::ModNamed(_)
        ));
        self.ui.page = self.ui.mods.enter(&to);
        self.game_packs.confirm_remove = None;
        self.game_packs.stale();
        self.mods_refresh();
        self.finish_page_change(before);
    }
}

/// A place in Installed that another page opens.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Installed {
    /// Mods › Installed, with every mod listed and no filters.
    Mods,
    /// Mods › Installed, the mods that are turned off.
    ModsOff,
    /// Mods › Installed with every mod listed, at the mod with this ID.
    Mod(String),
    /// Mods › Installed, the mods a search for this name finds.
    ModNamed(String),
    /// Packs › Installed, the shelf of this kind with all its packs.
    Shelf(Kind),
    /// Packs › Installed on the shelf of this kind with all its packs, at the pack that is
    /// this file: two packs may share a name, never a file.
    Pack(Kind, String),
    /// Packs › Installed on the shelf of this kind, the packs a search for this name finds.
    PackNamed(Kind, String),
}

/// The row Installed was opened at: it is scrolled to once and stands out.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Reveal {
    Mod(String),
    /// A pack, by its kind and file.
    Pack(Kind, String),
}

impl Reveal {
    pub(super) fn is_pack(&self, pack: &sunrise_launcher::packs::Pack) -> bool {
        matches!(self, Reveal::Pack(kind, file) if *kind == pack.kind && *file == pack.file)
    }
}

impl ModsState {
    /// Starts Installed afresh at `to`: the page to show.
    pub(crate) fn enter(&mut self, to: &Installed) -> Page {
        self.selection.clear();
        self.anchor = None;
        self.menu = None;
        self.unfit_open = None;
        self.have.set(String::new());
        self.row_filter = model::RowFilter::default();
        self.reveal = None;
        self.revealed.set(false);
        match to {
            Installed::Mods => {}
            Installed::ModsOff => self.row_filter.show = model::Show::Off,
            Installed::Mod(id) => self.reveal = Some(Reveal::Mod(id.clone())),
            Installed::Shelf(_) => {}
            Installed::Pack(kind, file) => self.reveal = Some(Reveal::Pack(*kind, file.clone())),
            Installed::ModNamed(title) | Installed::PackNamed(_, title) => {
                self.have.set(title.clone());
                self.row_filter.text = title.clone();
            }
        }
        let (Installed::Shelf(kind) | Installed::Pack(kind, _) | Installed::PackNamed(kind, _)) =
            to
        else {
            self.last_mods = Some(Page::Installed);
            return Page::Installed;
        };
        self.shelf = match kind {
            Kind::Resource => Shelf::Resource,
            Kind::Shader => Shelf::Shader,
        };
        self.data.reread();
        self.last_packs = Some(Page::Packs);
        Page::Packs
    }
}

impl Launcher {}

#[cfg(test)]
mod tests {
    use super::*;
    use model::{Local, Make, RowFilter, Show};

    /// Three mods put there by hand: `lantern` and `rain` on, `trail` off.
    fn rows() -> Vec<model::Row> {
        let local = ["lantern", "rain", "trail"].map(|id| Local {
            id: id.into(),
            file: format!("{id}.smod"),
            name: id.into(),
            version: "1.0".into(),
            github: None,
            native: id != "rain",
            served_by: None,
            unreadable: false,
        });
        let on: HashSet<String> = ["lantern", "rain"].map(String::from).into();
        model::rows(
            &[],
            &model::World {
                v2: None,
                v2_failure: None,
                build: None,
                unfit_updates: &Default::default(),
                enabled: &on,
                unbuilt: &HashSet::new(),
                chosen: &Default::default(),
                local: &local,
            },
        )
    }

    /// Installed as the player left it: two mods ticked (one of them on), a range begun,
    /// and filters that hide every mod that is off.
    fn left_behind() -> ModsState {
        let mut m = ModsState {
            selection: ["lantern", "trail"].map(String::from).into(),
            anchor: Some("sunrise-mods/trail.smod".into()),
            row_filter: RowFilter {
                text: "lantern".into(),
                show: Show::On,
                makes: [Make::Content].into(),
            },
            ..ModsState::default()
        };
        m.have.set("lantern".into());
        m
    }

    fn shown(m: &ModsState) -> Vec<String> {
        let rows = rows();
        model::visible(&rows, &m.row_filter)
            .into_iter()
            .map(|i| rows[i].id.clone())
            .collect()
    }

    fn nothing_carried_in(m: &ModsState) {
        assert!(m.selection.is_empty(), "no hidden row stays ticked");
        assert_eq!(m.anchor, None);
        assert!(m.row_filter.makes.is_empty());
    }

    #[test]
    fn turned_off_shows_the_mods_that_are_off_and_keeps_none_ticked() {
        let mut m = left_behind();
        assert!(shown(&m).is_empty(), "the old filters hid what is off");
        assert!(m.enter(&Installed::ModsOff) == Page::Installed);
        nothing_carried_in(&m);
        assert_eq!(m.row_filter.show, Show::Off);
        assert_eq!((m.have.text(), m.row_filter.text.as_str()), ("", ""));
        assert_eq!(shown(&m), ["trail"]);
    }

    #[test]
    fn manage_mods_does_not_keep_the_turned_off_shortcuts_filter() {
        let mut m = left_behind();
        m.enter(&Installed::ModsOff);
        assert_eq!(shown(&m), ["trail"]);
        assert!(m.enter(&Installed::Mods) == Page::Installed);
        assert_eq!(shown(&m), ["lantern", "rain", "trail"]);
        assert_eq!(m.row_filter, RowFilter::default());
    }

    #[test]
    fn a_mod_is_opened_by_its_id_among_all_of_them() {
        // Left on "Turned off" with another kind chosen: a mod that is on would be hidden.
        let mut m = left_behind();
        m.row_filter = RowFilter {
            text: String::new(),
            show: Show::Off,
            makes: [Make::Content].into(),
        };
        assert!(!shown(&m).contains(&"rain".to_string()));
        assert!(m.enter(&Installed::Mod("rain".into())) == Page::Installed);
        nothing_carried_in(&m);
        assert_eq!(m.row_filter, RowFilter::default());
        assert_eq!(shown(&m), ["lantern", "rain", "trail"]);
        assert_eq!(m.reveal, Some(Reveal::Mod("rain".into())));
        assert!(!m.revealed.get(), "it is scrolled to when next drawn");
    }

    #[test]
    fn a_search_by_name_starts_from_no_ticks_and_no_other_filter() {
        let mut m = left_behind();
        m.enter(&Installed::ModNamed("trail".into()));
        nothing_carried_in(&m);
        assert_eq!((m.have.text(), m.row_filter.show), ("trail", Show::All));
        assert_eq!(shown(&m), ["trail"]);
        assert_eq!(m.reveal, None);
    }

    #[test]
    fn home_destinations_open_the_requested_installed_filter_or_shelf() {
        // Wherever Mods and Packs were left: Packs on Discover, its Installed on the data
        // packs' shelf, a name typed that matches nothing.
        let left = || {
            let mut m = left_behind();
            m.shelf = Shelf::Data;
            m.last_packs = Some(Page::PackFind);
            m.have.set("zz".into());
            m.row_filter.text = "zz".into();
            m
        };
        let mut m = left();
        assert!(m.enter(&Installed::ModsOff) == Page::Installed);
        assert_eq!(shown(&m), ["trail"], "the mods that are off");
        for (kind, shelf) in [
            (Kind::Resource, Shelf::Resource),
            (Kind::Shader, Shelf::Shader),
        ] {
            let mut m = left();
            assert!(m.enter(&Installed::Shelf(kind)) == Page::Packs, "Installed");
            assert_eq!(m.shelf, shelf, "the requested shelf");
            assert_eq!(shelf.kind(), Some(kind));
            // Every pack of the shelf is listed: no name is searched for.
            assert_eq!((m.have.text(), m.row_filter.text.as_str()), ("", ""));
            assert!(m.last_packs == Some(Page::Packs));
            nothing_carried_in(&m);
        }
    }

    #[test]
    fn a_pack_is_opened_by_its_file_not_by_a_name_two_may_share() {
        let pack = |file: &str| sunrise_launcher::packs::Pack {
            kind: Kind::Shader,
            file: file.into(),
            path: file.into(),
            title: "Evening Glow".into(),
            description: String::new(),
            icon: None,
            enabled: file == "evening-glow-2.sshader",
        };
        let shelf = [
            pack("evening-glow-1.sshader"),
            pack("evening-glow-2.sshader"),
        ];
        let mut m = left_behind();
        m.shelf = Shelf::Data;
        let to = Installed::Pack(Kind::Shader, "evening-glow-2.sshader".into());
        assert!(m.enter(&to) == Page::Packs);
        nothing_carried_in(&m);
        assert_eq!(m.shelf, Shelf::Shader);
        // Both stay listed, as no name is searched for, and only the one asked for stands out.
        assert_eq!(m.have.text(), "");
        let reveal = m.reveal.clone().unwrap();
        assert_eq!(shelf.each_ref().map(|p| reveal.is_pack(p)), [false, true]);
        // The same file name on the other shelf is another pack.
        let resource = sunrise_launcher::packs::Pack {
            kind: Kind::Resource,
            ..shelf[1].clone()
        };
        assert!(!reveal.is_pack(&resource));
    }
}
