//! The Play bar's and Home's views of the profile's installed mods and packs: the rows the
//! Mods and Worlds tabs draw, and the refresh that keeps them to the files.
use super::*;
use crate::mods::selection::{bar, ModOp};

impl Launcher {
    pub(super) fn open_home_item(&mut self, open: Open) {
        match open {
            Open::Project(id) => self.mods_show_project(id),
            Open::Installed(id) => self.open_installed(crate::mods::Installed::Mod(id)),
            Open::Pack(kind, file) => self.open_installed(crate::mods::Installed::Pack(kind, file)),
        }
    }

    /// Refresh what the selected profile has installed off-thread. Home and the status strip
    /// read the same answer; no launch summary reads a file while drawing.
    pub(crate) fn refresh_home_inventory(&mut self, cx: &mut Context<Self>) {
        let Some(instance) = self.selected_instance() else {
            if let Some(old) = self.ui.home.mods.snapshot.take() {
                retire(&old, &mut self.pictures);
            }
            // Invalidate any answer for the previously selected profile.
            if self.ui.home.mods.query.take().is_some() {
                self.ui.home.mods.generation += 1;
            }
            return;
        };
        let query = Query {
            home: instance.home.clone(),
            version: self.version_of(instance).cloned(),
        };
        let mods = &mut self.ui.home.mods;
        if mods.query.as_ref() != Some(&query) {
            if let Some(old) = mods.snapshot.take() {
                retire(&old, &mut self.pictures);
            }
        }
        if let Some((generation, known)) = mods.draw(&query, Instant::now()) {
            let task = cx
                .background_executor()
                .spawn(async move { read_changed(&query, &known) });
            cx.spawn(async move |this, cx| {
                let read = task.await;
                let _ = this.update(cx, |this, cx| {
                    if this.ui.home.mods.generation != generation {
                        return;
                    }
                    let changed = read.is_some();
                    if let Some(old) = this.ui.home.mods.took(generation, read) {
                        retire(&old, &mut this.pictures);
                    }
                    if changed {
                        cx.notify();
                    }
                });
            })
            .detach();
        }
        if !std::mem::replace(&mut self.ui.home.mods.watched, true) {
            // While Home shows, its files are looked at now and then, never while drawing: a
            // change made elsewhere (the game, the command line) reaches the column.
            cx.spawn(async move |this, cx| loop {
                cx.background_executor().timer(WATCH).await;
                let Ok(shown) = this.update(cx, |this, _| {
                    let mods = &this.ui.home.mods;
                    if this.ui.page != Page::Home || mods.reading {
                        return None;
                    }
                    Some((mods.query.clone()?, mods.generation, mods.stamp.clone()))
                }) else {
                    break;
                };
                let Some((query, generation, known)) = shown else {
                    continue;
                };
                let now = cx
                    .background_executor()
                    .spawn(async move { stamp(&query) })
                    .await;
                if now != known {
                    let _ = this.update(cx, |this, cx| {
                        if this.ui.home.mods.generation == generation {
                            this.ui.home.mods.stale = true;
                            cx.notify();
                        }
                    });
                }
            })
            .detach();
        }
    }

    /// Home reads the same row facts as Installed, once per inventory or registry change. Each
    /// mod or pack is a card as the Worlds tab has them, and Add file stands where Worlds has
    /// Add world.
    pub(crate) fn home_installed_tab(&mut self, packs: bool, cx: &mut Context<Self>) -> Content {
        let add = self.add_file_button(
            packs,
            if packs {
                "home-packs-add"
            } else {
                "home-mods-add"
            },
            cx,
        );
        let Some(snapshot) = self.ui.home.mods.snapshot.clone() else {
            return Content {
                count: None,
                manage: Some(add),
                items: div(),
            };
        };
        let full = if packs {
            &snapshot.tabs.resources
        } else {
            &snapshot.tabs.all_mods
        };
        if full.is_empty() {
            return Content {
                count: Some(0),
                // The empty state has its own Add file.
                manage: None,
                items: self.home_installed_empty(packs, cx),
            };
        }
        // The rows as the screen has them: a switch just flipped is already in its new place.
        self.home_settle_flips(cx);
        let all: Vec<Item> = full
            .iter()
            .map(|item| self.home_flipped(packs, item))
            .collect();
        let count = all.len();
        let query = self.ui.home.tabs.query(packs);
        let list: Vec<Item> = all
            .iter()
            .filter(|item| home_item_matches(item, &query))
            .cloned()
            .collect();
        // Rows already gone drop out of the ticks; the rest keep them as the list changes.
        let every: Vec<String> = all.iter().map(|item| item.id.clone()).collect();
        self.home_pick(packs).retain(&every);
        if list.is_empty() {
            return Content {
                count: Some(count),
                manage: Some(add),
                items: tabs::no_matches(),
            };
        }
        let order: std::rc::Rc<Vec<String>> =
            std::rc::Rc::new(list.iter().map(|item| item.id.clone()).collect());
        let rows = self.home_cards(packs, &list, &all, &order, cx);
        let pick_bar = self.home_pick_bar(packs, &all, &order, cx);
        self.home_set_bar(pick_bar);
        Content {
            count: Some(count),
            manage: Some(add),
            items: rows,
        }
    }

    /// `item` as the card shows it: on or off as the player just flipped it, until the files
    /// say so themselves.
    fn home_flipped(&self, packs: bool, item: &Item) -> Item {
        let mut item = item.clone();
        if let Some(flip) = self.ui.home.tabs.flipped.get(&(packs, item.id.clone())) {
            item.on = flip.on;
        }
        item
    }

    /// Drops the flips the files have answered (a new inventory) or never will (the change
    /// ended and nothing came of it, as when it failed): the card shows the files again. While a
    /// flip waits for its answer the inventory is looked at once more.
    fn home_settle_flips(&mut self, cx: &mut Context<Self>) {
        if self.ui.home.tabs.flipped.is_empty() {
            return;
        }
        let revision = self.ui.home.mods.revision;
        let busy = self.mutating();
        let now = Instant::now();
        let mut waiting = false;
        self.ui.home.tabs.flipped.retain(|_, flip| {
            if flip.revision != revision {
                return false;
            }
            if busy {
                return true;
            }
            let done = *flip.done.get_or_insert(now);
            waiting = true;
            now.duration_since(done) < SETTLE
        });
        if waiting {
            self.ui.home.mods.stale = true;
            cx.notify();
        }
    }

    /// The cards of the rows the list has room to build now. `list` is what the search leaves
    /// of `all`.
    fn home_cards(
        &mut self,
        packs: bool,
        list: &[Item],
        all: &[Item],
        order: &std::rc::Rc<Vec<String>>,
        cx: &mut Context<Self>,
    ) -> Div {
        let revision = self.ui.home.mods.revision;
        let facts = (!packs).then(|| {
            (
                self.home_mod_facts(revision),
                self.home_card_facts(revision),
            )
        });
        let (built, place) = self.home_installed_list(packs, list.len());
        // No catalogue read yet this run: fetch it so registry mods get their icons, as the Mods
        // pages do.
        if !packs
            && sunrise::v2::shown_index().is_none()
            && list[built.clone()]
                .iter()
                .any(|item| matches!(item.picture, Picture::Project(_)))
        {
            self.ensure_v2_catalog(false);
        }
        // A pack's place in the stack is its place among the packs that are on, the whole list
        // over, not among those the search leaves.
        let mut before: HashMap<&str, usize> = HashMap::new();
        if packs {
            let mut on = 0;
            for item in all {
                before.insert(item.id.as_str(), on);
                on += usize::from(item.on);
            }
        }
        let ticked = self.home_pick(packs).clone();
        let mut rows = div().flex().flex_col().gap(px(tabs::GAP));
        for item in &list[built] {
            let (detail, switchable) = match &facts {
                None => {
                    let mut priority = before.get(item.id.as_str()).copied().unwrap_or(0);
                    (Self::home_pack_detail(item, &mut priority), true)
                }
                Some((labels, facts)) => {
                    let fact = facts.get(&item.id);
                    (
                        Self::home_mod_detail(item, labels.get(&item.id), fact),
                        fact.is_none_or(|fact| fact.switchable),
                    )
                }
            };
            let open = item.open.clone();
            let card = tabs::Card {
                id: format!("home-item:{:?}:{}", item.kind, item.id).into(),
                picture: self.home_item_picture(item, cx),
                title: item.name.clone(),
                detail,
                on: item.on,
                switchable,
                pick: tabs::Pick {
                    packs,
                    key: item.id.clone(),
                    order: order.clone(),
                    ticked: ticked.contains(&item.id),
                    any: !ticked.is_empty(),
                },
            };
            rows = rows
                .child(self.home_card(card, cx, move |this, _| this.open_home_item(open.clone())));
        }
        place(rows)
    }

    /// The bar under the list while rows are ticked, offering what applies to them.
    fn home_pick_bar(
        &mut self,
        packs: bool,
        list: &[Item],
        order: &std::rc::Rc<Vec<String>>,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let ticked = self.home_pick(packs).clone();
        if ticked.is_empty() {
            return None;
        }
        let all = ticked.all_of(order);
        let toggle = bar::link(
            "home-pick-all",
            if all { tr("Clear") } else { tr("Select all") },
        )
        .on_click({
            let order = order.clone();
            cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.home_pick(packs).toggle_all(&order);
                cx.notify();
            })
        });
        let mut actions: Vec<AnyElement> = Vec::new();
        // What the ticked rows allow: a pack's state is on the row; a mod's comes from Installed.
        let (on, off, updates, removable) = if packs {
            let count = |on: bool| {
                list.iter()
                    .filter(|item| item.on == on && ticked.contains(&item.id))
                    .count()
            };
            (count(true), count(false), 0, ticked.len())
        } else {
            let facts = self.mod_selection_facts(ticked.keys());
            if !facts.ready {
                return Some(bar::bar(ticked.len(), toggle, actions));
            }
            (facts.on, facts.off, facts.updates, facts.removable)
        };
        let mut push = |id: &'static str, label: String, op: ModOp, destructive: bool| {
            actions.push(
                bar::action(id, label, true, destructive)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.home_pick_run(packs, op, cx);
                        cx.notify();
                    }))
                    .into_any_element(),
            );
        };
        if off > 0 {
            push(
                "home-pick-enable",
                tr("Enable").into(),
                ModOp::Enable,
                false,
            );
        }
        if on > 0 {
            push(
                "home-pick-disable",
                tr("Disable").into(),
                ModOp::Disable,
                false,
            );
        }
        if updates > 0 {
            push(
                "home-pick-update",
                trf("Update {count}", &[("count", &updates.to_string())]),
                ModOp::Update,
                false,
            );
        }
        if removable > 0 {
            let count = ticked.len().to_string();
            let label = if ticked.removal_armed() {
                trf("Confirm: remove {count}", &[("count", &count)])
            } else {
                trf("Remove {count}…", &[("count", &count)])
            };
            push("home-pick-remove", label, ModOp::Remove, true);
        }
        Some(bar::bar(ticked.len(), toggle, actions))
    }

    /// Does `op` to the ticked rows as one change. Remove asks once first: the same press on
    /// the same ticks confirms it.
    fn home_pick_run(&mut self, packs: bool, op: ModOp, cx: &mut Context<Self>) {
        let keys = self.home_pick(packs).keys().clone();
        if keys.is_empty() || (op == ModOp::Remove && !self.home_pick(packs).confirm_removal()) {
            return;
        }
        if packs {
            let Some(home) = self
                .selected_instance()
                .map(|instance| instance.home.clone())
            else {
                return;
            };
            let op = match op {
                ModOp::Enable => crate::mods::selection::PackOp::Enable,
                ModOp::Disable => crate::mods::selection::PackOp::Disable,
                ModOp::Remove => crate::mods::selection::PackOp::Remove,
                ModOp::Update => return,
            };
            self.run_pack_op(home, PackKind::Resource, keys.into_iter().collect(), op, cx);
        } else {
            self.run_mod_op(op, &keys);
        }
        self.home_pick(packs).clear();
    }

    /// The ids of the rows the Mods or Packs tab shows now (its search applied), in order.
    pub(crate) fn home_list_ids(&self, packs: bool) -> Vec<String> {
        let Some(snapshot) = &self.ui.home.mods.snapshot else {
            return Vec::new();
        };
        let list = if packs {
            &snapshot.tabs.resources
        } else {
            &snapshot.tabs.all_mods
        };
        let query = self.ui.home.tabs.query(packs);
        list.iter()
            .filter(|item| home_item_matches(item, &query))
            .map(|item| item.id.clone())
            .collect()
    }

    /// Something other than the Home page changed the profile's mods or packs: read them again.
    pub(crate) fn home_inventory_changed(&mut self) {
        self.ui.home.mods.stale = true;
    }
}

/// Whether `item` is what the search `query` (lower-cased) asks for: its name, version or
/// source.
fn home_item_matches(item: &Item, query: &str) -> bool {
    let source = item.manual_badge.map_or("", |badge| badge.label());
    tabs::matches_query(
        query,
        &[
            item.name.as_str(),
            item.id.as_str(),
            item.mod_version.as_deref().unwrap_or(""),
            source,
        ],
    )
}

/// How long a flipped switch waits for the files to say so after its change ended.
const SETTLE: Duration = Duration::from_millis(1500);
