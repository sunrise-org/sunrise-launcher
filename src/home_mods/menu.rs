//! A card's "…" menu on the Mods and Packs tabs: which rows it has for the mod or pack, and
//! what each does. A change goes through the same actions and locks as the Mods pages, and its
//! refusal or failure is told in a toast.
use super::*;
use crate::home::tabs::{ItemAction, MenuEntry};
use crate::mods::selection::PackOp;

impl Launcher {
    fn home_item(&self, packs: bool, key: &str) -> Option<Item> {
        let snapshot = self.ui.home.mods.snapshot.as_ref()?;
        let list = if packs {
            &snapshot.tabs.resources
        } else {
            &snapshot.tabs.all_mods
        };
        list.iter().find(|item| item.id == key).cloned()
    }

    /// The folder of the selected profile.
    fn home_item_home(&self) -> Option<PathBuf> {
        self.selected_instance()
            .map(|instance| instance.home.clone())
    }

    /// The card's switch: turns the mod or pack on or off. Nothing is said when it works (the
    /// switch shows it); a refusal or a failure is.
    pub(crate) fn home_item_toggle(&mut self, packs: bool, key: &str, cx: &mut Context<Self>) {
        if !packs {
            self.home_mod_toggle(key);
            return;
        }
        let (Some(item), Some(home)) = (self.home_item(true, key), self.home_item_home()) else {
            return;
        };
        let op = if item.on {
            PackOp::Disable
        } else {
            PackOp::Enable
        };
        self.run_pack_change(home, PackKind::Resource, vec![item.id], op, false, cx);
    }

    /// A row of the menu that does `action`, as it is until a lock or a rule dims it.
    fn menu_row(
        id: &'static str,
        icon: &'static str,
        label: &'static str,
        action: ItemAction,
    ) -> MenuEntry {
        MenuEntry {
            id,
            icon,
            label: label.into(),
            danger: false,
            off: None,
            action,
        }
    }

    /// The rows of the menu of the card `key`, only those that apply. While the game runs the
    /// rows that change something stay, dimmed, with the reason as their tooltip.
    pub(crate) fn home_item_entries(&mut self, packs: bool, key: &str) -> Vec<MenuEntry> {
        let Some(item) = self.home_item(packs, key) else {
            return Vec::new();
        };
        let locked: Option<SharedString> = self.mods_locked().map(Into::into);
        // Rows that change the item: dimmed while locked.
        let change = |entry: MenuEntry| MenuEntry {
            off: locked.clone(),
            ..entry
        };
        let mut rows = vec![Self::menu_row(
            "home-menu-details",
            "information-circle",
            tr("Details"),
            ItemAction::Details,
        )];
        let remove = if packs {
            rows.push(change(self.home_toggle_entry(&item, true)));
            let folder = self
                .home_item_home()
                .map(|home| home.join(PackKind::Resource.folder()).join(&item.id));
            rows.extend(folder.map(Self::folder_entry));
            true
        } else {
            let facts = self.home_card_facts(self.ui.home.mods.revision);
            let fact = facts.get(&item.id);
            if fact.is_none_or(|fact| fact.switchable) {
                rows.push(change(self.home_toggle_entry(&item, false)));
            }
            if fact.is_some_and(|fact| fact.update.is_some()) {
                rows.push(change(Self::menu_row(
                    "home-menu-update",
                    "refresh",
                    tr("Update"),
                    ItemAction::Update,
                )));
            }
            rows.extend(
                fact.and_then(|fact| fact.file.clone())
                    .map(Self::folder_entry),
            );
            rows.push(Self::menu_row(
                "home-menu-copy",
                "copy-01",
                tr("Copy name"),
                ItemAction::CopyName(item.name.clone()),
            ));
            if let Some(page) = fact.and_then(|fact| fact.page.clone()) {
                rows.push(Self::menu_row(
                    "home-menu-page",
                    "link-01",
                    tr("Registry page"),
                    ItemAction::OpenPage(page),
                ));
            }
            fact.is_none_or(|fact| fact.removable)
        };
        if remove {
            rows.push(change(MenuEntry {
                danger: true,
                ..Self::menu_row(
                    "home-menu-remove",
                    "delete-02",
                    tr("Remove…"),
                    ItemAction::Remove,
                )
            }));
        }
        rows
    }

    fn folder_entry(path: PathBuf) -> MenuEntry {
        Self::menu_row(
            "home-menu-folder",
            "folder-01",
            tr("Show in folder"),
            ItemAction::ShowFile(path),
        )
    }

    /// Enable or Disable, by what the item is now.
    fn home_toggle_entry(&self, item: &Item, packs: bool) -> MenuEntry {
        let (id, label) = match (packs, item.on) {
            (false, true) => ("home-menu-off", tr("Disable")),
            (false, false) => ("home-menu-on", tr("Enable")),
            (true, true) => ("home-menu-pack-off", tr("Disable")),
            (true, false) => ("home-menu-pack-on", tr("Enable")),
        };
        MenuEntry {
            id,
            icon: if item.on { "view-off-slash" } else { "view" },
            label: label.into(),
            danger: false,
            off: None,
            action: ItemAction::Toggle,
        }
    }

    /// Does what a row of the card's menu says.
    pub(crate) fn home_item_run(
        &mut self,
        packs: bool,
        key: &str,
        action: ItemAction,
        cx: &mut Context<Self>,
    ) {
        match action {
            ItemAction::Details => {
                if let Some(item) = self.home_item(packs, key) {
                    self.ui.home.tabs.remember(None);
                    self.open_home_item(item.open);
                }
            }
            ItemAction::Toggle => self.home_item_toggle(packs, key, cx),
            ItemAction::Update => self.home_mod_update(key),
            ItemAction::ShowFile(path) => cx.reveal_path(&path),
            ItemAction::CopyName(name) => {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(name));
            }
            ItemAction::OpenPage(url) => Launcher::open_url(&url, cx),
            ItemAction::Remove if packs => self.home_pack_remove(key),
            ItemAction::Remove => self.home_mod_remove(key),
        }
    }

    /// Remove… on a pack: ticks it alone and asks once, so the selection bar offers "Confirm:
    /// remove 1", as it does for ticked packs.
    fn home_pack_remove(&mut self, key: &str) {
        let order = self.home_list_ids(true);
        let picks = self.home_pick(true);
        picks.clear();
        picks.click(&order, key, false);
        picks.confirm_removal();
    }
}
