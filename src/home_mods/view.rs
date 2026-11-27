//! The Play bar's and Home's views of the profile's installed mods and packs: the rows the
//! Mods and Worlds tabs draw, and the refresh that keeps them to the files.
use super::*;
use crate::mods::selection::{bar, ModOp};

impl Launcher {
    fn open_home_item(&mut self, open: Open) {
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

    /// Home reads the same row facts as Installed, once per inventory or registry change.
    pub(crate) fn home_installed_tab(&mut self, packs: bool, cx: &mut Context<Self>) -> Content {
        use crate::mods::Installed;
        let manage = if packs {
            secondary("home-manage-resource", tr("Manage resource packs")).on_click(cx.listener(
                |this, _, _, cx| {
                    this.open_installed(Installed::Shelf(PackKind::Resource));
                    cx.notify();
                },
            ))
        } else {
            secondary("home-manage-mods", tr("Manage mods")).on_click(cx.listener(
                |this, _, _, cx| {
                    this.open_installed(Installed::Mods);
                    cx.notify();
                },
            ))
        };
        let Some(snapshot) = self.ui.home.mods.snapshot.clone() else {
            return Content {
                count: None,
                turned_off: 0,
                manage: Some(manage),
                items: div(),
            };
        };
        let list = if packs {
            &snapshot.tabs.resources
        } else {
            &snapshot.tabs.mods
        };
        let turned_off = if packs { 0 } else { snapshot.tabs.mods_off };
        if list.is_empty() {
            let items = if packs {
                tabs::empty(
                    tr("No resource packs yet"),
                    tr("Resource packs you add to this profile show up here."),
                    Some(
                        secondary("home-find-packs", tr("Find resource packs")).on_click(
                            cx.listener(|this, _, _, cx| {
                                this.show_resource_pack_discover();
                                cx.notify();
                            }),
                        ),
                    ),
                )
            } else if turned_off == 0 {
                tabs::empty(
                    tr("No mods yet"),
                    tr("Mods you add to this profile show up here."),
                    Some(
                        secondary("home-browse-mods", tr("Find mods")).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.show_discover();
                                cx.notify();
                            },
                        )),
                    ),
                )
            } else {
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(crate::home::muted(tr("No mods turned on")))
            };
            return Content {
                count: Some(0),
                turned_off,
                manage: (turned_off > 0).then_some(manage),
                items,
            };
        }
        let mod_facts = (!packs).then(|| self.home_mod_facts(self.ui.home.mods.revision));
        let (built, place) = self.home_installed_list(packs, list.len());
        let index = (!packs).then(sunrise::v2::shown_index).flatten();
        // No catalogue read yet this run: fetch it so registry mods get their icons, as the Mods
        // pages do.
        if !packs
            && index.is_none()
            && list[built.clone()]
                .iter()
                .any(|item| matches!(item.picture, Picture::Project(_)))
        {
            self.ensure_v2_catalog(false);
        }
        // Rows already gone drop out of the ticks; the rest keep them as the list changes.
        let order: std::rc::Rc<Vec<String>> =
            std::rc::Rc::new(list.iter().map(|item| item.id.clone()).collect());
        self.home_pick(packs).retain(&order);
        let ticked = self.home_pick(packs).clone();
        let mut rows = div().flex().flex_col();
        let mut priority = list[..built.start].iter().filter(|item| item.on).count();
        for item in &list[built] {
            let picture = match &item.picture {
                Picture::Project(project) => tile(
                    index
                        .as_ref()
                        .and_then(|catalog| Some(catalog.index.project(project)?.icon.clone()))
                        .and_then(|icon| {
                            self.v2_mod_image(
                                project,
                                &icon,
                                sunrise_launcher::browser::images::ROW,
                            )
                        }),
                    &item.id,
                    &item.name,
                    PICTURE,
                ),
                Picture::Local { package, source } => {
                    let listing = if let Some(origin) = source {
                        self.read_nightly_catalogue(false);
                        self.source_listing(origin)
                    } else {
                        None
                    };
                    if let Some(source) = listing {
                        self.source_tile(
                            source,
                            None,
                            self.nightly_catalogue_current(),
                            package.clone(),
                            PICTURE,
                            cx,
                        )
                    } else {
                        tile(package.clone(), &item.id, &item.name, PICTURE)
                    }
                }
                Picture::Pack(art) => tile(art.show(cx), &item.id, &item.name, PICTURE),
            };
            let (detail, badge, state) = if packs {
                let status = if item.on {
                    priority += 1;
                    trf(
                        "Enabled · {priority}",
                        &[("priority", &priority.to_string())],
                    )
                } else {
                    tr("Off").to_owned()
                };
                (None, None, Some(status))
            } else {
                let fact = mod_facts.as_ref().and_then(|facts| facts.get(&item.id));
                let kind = fact.map(|fact| fact.kind).filter(|kind| !kind.is_empty());
                let badge = fact.and_then(|fact| fact.badge).or(item.manual_badge);
                (
                    kind.map(str::to_owned),
                    badge,
                    fact.and_then(|fact| fact.status.clone()),
                )
            };
            let open = item.open.clone();
            let pick = tabs::Pick {
                packs,
                key: item.id.clone(),
                order: order.clone(),
                ticked: ticked.contains(&item.id),
                any: !ticked.is_empty(),
            };
            rows = rows.child(self.home_pick_row(
                tabs::RowDisplay {
                    id: format!("home-item:{:?}:{}", item.kind, item.id).into(),
                    picture: picture.into_any_element(),
                    title: item.name.clone(),
                    detail,
                    badge,
                    tail: state,
                },
                pick,
                cx,
                move |this, _| this.open_home_item(open.clone()),
            ));
        }
        let pick_bar = self.home_pick_bar(packs, list, &order, cx);
        self.home_set_bar(pick_bar);
        Content {
            count: Some(list.len()),
            turned_off,
            manage: Some(manage),
            items: place(rows),
        }
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

    /// The ids of the rows the Mods or Packs tab lists, in order.
    pub(crate) fn home_list_ids(&self, packs: bool) -> Vec<String> {
        let Some(snapshot) = &self.ui.home.mods.snapshot else {
            return Vec::new();
        };
        let list = if packs {
            &snapshot.tabs.resources
        } else {
            &snapshot.tabs.mods
        };
        list.iter().map(|item| item.id.clone()).collect()
    }

    /// Something other than the Home page changed the profile's mods or packs: read them again.
    pub(crate) fn home_inventory_changed(&mut self) {
        self.ui.home.mods.stale = true;
    }
}
