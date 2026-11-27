//! The Play bar's and Home's views of the profile's installed mods and packs: the rows the
//! Mods and Worlds tabs draw, and the refresh that keeps them to the files.
use super::*;

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
            rows = rows.child(self.home_overview_row(
                tabs::RowDisplay {
                    id: format!("home-item:{:?}:{}", item.kind, item.id).into(),
                    picture: picture.into_any_element(),
                    title: item.name.clone(),
                    detail,
                    badge,
                    tail: state,
                },
                cx,
                move |this, _| this.open_home_item(open.clone()),
            ));
        }
        Content {
            count: Some(list.len()),
            turned_off,
            manage: Some(manage),
            items: place(rows),
        }
    }
}
