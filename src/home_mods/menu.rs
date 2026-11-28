//! A card's "…" menu on the Mods and Packs tabs: which rows it has for the mod or pack, and
//! what each does. A change goes through the same actions and locks as the Mods pages, and its
//! refusal or failure is told in a toast.
use super::*;
use crate::home::tabs::{Flip, ItemAction, MenuEntry};
use crate::mods::selection::PackOp;

impl Launcher {
    fn home_item(&self, packs: bool, key: &str) -> Option<Item> {
        let snapshot = self.ui.home.mods.snapshot.as_ref()?;
        let list = if packs {
            &snapshot.tabs.resources
        } else {
            &snapshot.tabs.all_mods
        };
        let mut item = list.iter().find(|item| item.id == key).cloned()?;
        // A switch just flipped counts as flipped, as its card shows it.
        if let Some(flip) = self.ui.home.tabs.flipped.get(&(packs, key.to_owned())) {
            item.on = flip.on;
        }
        Some(item)
    }

    /// The folder of the selected profile.
    fn home_item_home(&self) -> Option<PathBuf> {
        self.selected_instance()
            .map(|instance| instance.home.clone())
    }

    /// The card's switch: turns the mod or pack on or off. The card shows the flip at once; when
    /// the change fails the files bring it back. Nothing is said when it works (the switch shows
    /// it); a refusal or a failure is.
    pub(crate) fn home_item_toggle(&mut self, packs: bool, key: &str, cx: &mut Context<Self>) {
        let Some(item) = self.home_item(packs, key) else {
            return;
        };
        let was_busy = self.mutating();
        if packs {
            let Some(home) = self.home_item_home() else {
                return;
            };
            if let Some(why) = self.pack_refusal(&home) {
                crate::toast::say(why);
                return;
            }
            let op = if item.on {
                PackOp::Disable
            } else {
                PackOp::Enable
            };
            self.ui.home.tabs.quiet = true;
            self.run_pack_change(
                home,
                PackKind::Resource,
                vec![item.id.clone()],
                op,
                false,
                cx,
            );
        } else {
            self.home_mod_toggle(key);
        }
        // Only a change that began is shown ahead of the files; a refusal leaves the card be.
        if !was_busy && self.mutating() {
            let revision = self.ui.home.mods.revision;
            self.ui.home.mods.keep_order |= packs;
            self.ui.home.tabs.flipped.insert(
                (packs, item.id),
                Flip {
                    on: !item.on,
                    revision,
                    done: None,
                },
            );
            // A frame once the change should be over, to show the files' answer.
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(Duration::from_secs(2)).await;
                let _ = this.update(cx, |_, cx| cx.notify());
            })
            .detach();
        }
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
        // On and off is the card's own switch, so the menu leaves it out.
        let remove = if packs {
            let folder = self
                .home_item_home()
                .map(|home| home.join(PackKind::Resource.folder()).join(&item.id));
            rows.extend(folder.map(Self::folder_entry));
            true
        } else {
            let facts = self.home_card_facts(self.ui.home.mods.revision);
            let fact = facts.get(&item.id);
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
