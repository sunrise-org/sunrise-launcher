//! Data packs on Packs. A data pack lives in a world, so both sides are about worlds:
//!
//! - **Discover** adds one through the world picker ([`Pick`]): a sheet listing the setup's
//!   worlds with a box each, so one pack can go into several at once. Nothing is chosen for
//!   the player; a world that has the pack already says so.
//! - **Installed** lists every world of the setup with its data packs ([`DataList`]). A pack is
//!   removed there, or copied into other worlds through the same picker. Home's Data packs
//!   button on a world card lands here, with that world first.
//!
//! The game never says where a pack file came from, so what Discover added is noted in
//! `datapack-browser.json` beside `mod-browser.json` ([`Origin`]). It is only read for a file
//! that is still in the world, and is how a world is known to have a project already.
use super::pack_discover::check_world;
use super::ui::{added_mark, c, card, checkbox, dim, muted, primary, tile};
use super::*;
use crate::frame::button;
use crate::paint::palette;
use serde::{Deserialize, Serialize};
use std::path::Path;
use sunrise_launcher::browser::{self, Project};
use sunrise_launcher::datapacks::{self, DataPack};
use sunrise_launcher::worlds::World;

const RECORD: &str = "datapack-browser.json";

/// The project a data pack file in a world was downloaded from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct Origin {
    /// The world's folder under `saves/`, and the pack's file in its `datapacks/`.
    world: String,
    file: String,
    source: SourceId,
    project: String,
    title: String,
    number: String,
}

impl Origin {
    fn path(&self, home: &Path) -> PathBuf {
        home.join("saves")
            .join(&self.world)
            .join("datapacks")
            .join(&self.file)
    }
}

/// The notes of `home` whose file is still where it was put.
pub(super) fn origins(home: &Path) -> Vec<Origin> {
    let all: Vec<Origin> = std::fs::read(home.join(RECORD))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    all.into_iter().filter(|o| o.path(home).exists()).collect()
}

/// Adds `new` to the notes of `home`, each replacing an older note of the same file.
fn note_origins(home: &Path, new: Vec<Origin>) -> Result<(), String> {
    let mut all = origins(home);
    all.retain(|o| !new.iter().any(|n| n.world == o.world && n.file == o.file));
    all.extend(new);
    let text = serde_json::to_string_pretty(&all).map_err(|e| e.to_string())?;
    std::fs::write(home.join(RECORD), text).map_err(|e| format!("{RECORD}: {e}"))
}

/// One data pack of a world, with where it came from when Discover put it there.
#[derive(Clone)]
pub(super) struct Listed {
    pack: DataPack,
    origin: Option<Origin>,
}

impl Listed {
    /// Its project's name and version, else its file's name.
    fn title(&self) -> String {
        match &self.origin {
            Some(o) => format!("{} {}", o.title, o.number),
            None => self
                .pack
                .file
                .strip_suffix(".zip")
                .unwrap_or(&self.pack.file)
                .to_owned(),
        }
    }
}

/// Every world of one setup with its data packs, as last read.
pub(super) struct DataList {
    home: PathBuf,
    worlds: Vec<(World, Vec<Listed>)>,
}

/// What the picker adds.
#[derive(Clone)]
pub(super) enum What {
    /// A project from Discover: downloaded once, then put in each world.
    Project(Project),
    /// A pack file already in the world `from`, copied as it is.
    Copy {
        from: PathBuf,
        title: String,
        pack: DataPack,
        origin: Option<Origin>,
    },
}

impl What {
    fn title(&self) -> &str {
        match self {
            What::Project(p) => &p.title,
            What::Copy { title, .. } => title,
        }
    }

    /// Whether `world` has it already: the same project, or the very file.
    fn in_world(&self, world: &World, packs: &[Listed]) -> bool {
        let same = |source: SourceId, id: &str| {
            packs.iter().any(|l| {
                l.origin
                    .as_ref()
                    .is_some_and(|o| o.source == source && o.project == id)
            })
        };
        match self {
            What::Project(p) => same(p.source, &p.id),
            What::Copy {
                from, pack, origin, ..
            } => {
                *from == world.path
                    || packs.iter().any(|l| l.pack.file == pack.file)
                    || origin.as_ref().is_some_and(|o| same(o.source, &o.project))
            }
        }
    }
}

/// The world picker on screen: what it adds, and the worlds ticked so far.
pub(super) struct Pick {
    what: What,
    ticked: BTreeSet<PathBuf>,
}

#[derive(Default)]
pub(super) struct DataState {
    list: Option<DataList>,
    /// Read the worlds' folders again before the next frame.
    stale: bool,
    /// The world Home's shortcut opened Installed for: listed first.
    first: Option<PathBuf>,
    pick: Option<Pick>,
    /// The pack whose Remove was pressed once: its world and file.
    confirm: Option<(PathBuf, String)>,
}

impl DataState {
    /// Something changed in a world's folder.
    pub(super) fn reread(&mut self) {
        self.stale = true;
        self.confirm = None;
    }

    /// How many worlds have the project `id` of `source`.
    pub(super) fn worlds_with(&self, source: SourceId, id: &str) -> usize {
        let Some(list) = &self.list else { return 0 };
        list.worlds
            .iter()
            .filter(|(_, packs)| {
                packs.iter().any(|l| {
                    l.origin
                        .as_ref()
                        .is_some_and(|o| o.source == source && o.project == id)
                })
            })
            .count()
    }
}

/// A world's own picture, or its first letter.
fn world_icon(world: &World, side: f32) -> Div {
    tile(world.icon.clone(), &world.folder, &world.name, side)
}

/// "Survival · Yesterday".
fn world_facts(world: &World) -> String {
    format!(
        "{} · {}",
        tr(world.mode),
        crate::home::sentence(&crate::worlds_page::ago(world.last_played))
    )
}

/// "A", "A and B", "A, B and C".
pub(crate) fn names(list: &[String]) -> String {
    match list {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => trf(
            "{names} and {last}",
            &[("names", &rest.join(", ")), ("last", last)],
        ),
    }
}

impl Launcher {
    /// Reads the selected setup's worlds and their data packs when they aren't in memory, the
    /// setup changed, a world came or went, or something was added or removed.
    pub(super) fn data_list(&mut self, cx: &mut Context<Self>) {
        let home = self.mods_home();
        self.load_worlds(home.clone(), cx);
        let worlds: Vec<World> = match self.worlds.home.as_ref() == Some(&home) {
            true => self.worlds.list.clone(),
            false => Vec::new(),
        };
        let d = &mut self.ui.mods.data;
        let current = d.list.as_ref().is_some_and(|l| {
            l.home == home
                && l.worlds.len() == worlds.len()
                && l.worlds
                    .iter()
                    .zip(&worlds)
                    .all(|((a, _), b)| a.path == b.path)
        });
        if current && !d.stale {
            return;
        }
        d.stale = false;
        let notes = origins(&home);
        let worlds = worlds
            .into_iter()
            .map(|world| {
                let packs = datapacks::list(&world.path)
                    .into_iter()
                    .map(|pack| Listed {
                        origin: notes
                            .iter()
                            .find(|o| o.world == world.folder && o.file == pack.file)
                            .cloned(),
                        pack,
                    })
                    .collect();
                (world, packs)
            })
            .collect();
        d.list = Some(DataList { home, worlds });
    }

    /// Home's Data packs on a world card: Packs › Installed › Data packs, that world first.
    pub(crate) fn show_world_packs(&mut self, world: PathBuf) {
        self.leave_section(true);
        self.worlds.stale();
        let m = &mut self.ui.mods;
        m.shelf = Shelf::Data;
        m.data.first = Some(world);
        m.data.reread();
        m.last_packs = Some(Page::Packs);
        self.ui.page = Page::Packs;
    }

    /// Opens the world picker for `what`, with `hint` ticked when it can take it.
    pub(super) fn pick_worlds(&mut self, what: What, hint: Option<PathBuf>) {
        if !self.guard() {
            return;
        }
        let ticked = self
            .ui
            .mods
            .data
            .list
            .iter()
            .flat_map(|l| &l.worlds)
            .filter(|(w, packs)| Some(&w.path) == hint.as_ref() && !what.in_world(w, packs))
            .map(|(w, _)| w.path.clone())
            .collect();
        self.ui.mods.data.pick = Some(Pick { what, ticked });
        self.open_sheet(crate::sheets::Sheet::Worlds);
    }

    /// The picker's Add: everything is checked again, then done in a worker.
    fn pick_confirm(&mut self) {
        let Some(pick) = self.ui.mods.data.pick.take() else {
            return;
        };
        self.close_sheet();
        if !self.guard() {
            return;
        }
        let Some(instance) = self
            .selected_instance()
            .filter(|_| !self.location_unavailable)
            .cloned()
        else {
            self.ui.mods.notice = Some(Err(tr("Select a Sunrise profile first.").to_string()));
            return;
        };
        // Only worlds of the setup on screen, in the order they are listed.
        let worlds: Vec<World> = self
            .ui
            .mods
            .data
            .list
            .iter()
            .filter(|l| l.home == instance.home)
            .flat_map(|l| &l.worlds)
            .filter(|(w, packs)| pick.ticked.contains(&w.path) && !pick.what.in_world(w, packs))
            .map(|(w, _)| w.clone())
            .collect();
        if worlds.is_empty() {
            return;
        }
        self.dismiss_notice();
        self.ui.mods.busy = Some((tr("Adding the data pack…").into(), 0.));
        let (tx, game) = (self.ui.mods.tx.clone(), self.mods_game());
        std::thread::spawn(move || {
            let result = sunrise_launcher::managed_play::Profile::open_selected(&instance)
                .and_then(|profile| {
                    add_to_worlds(&pick.what, profile.home(), &worlds, &game, &|f| {
                        let _ = tx.send(Event::Progress(pick.what.title().to_owned(), f));
                    })
                });
            let _ = tx.send(Event::PackDone(result));
        });
    }

    /// Remove on a row: the second press moves the pack to the trash.
    fn data_remove(&mut self, world: PathBuf, file: String) {
        let key = (world, file);
        if self.ui.mods.data.confirm.as_ref() != Some(&key) {
            self.ui.mods.data.confirm = Some(key);
            return;
        }
        self.ui.mods.data.reread();
        if !self.guard() {
            return;
        }
        let (world, file) = key;
        let result = (|| {
            let instance = self
                .selected_instance()
                .ok_or_else(|| tr("Select a Sunrise profile first."))?;
            let profile = sunrise_launcher::managed_play::Profile::open_selected(instance)?;
            let home = profile.home();
            let _guard = browser::try_lock_home(home).ok_or_else(|| {
                tr("This profile is busy. Try again when the operation finishes.")
            })?;
            check_world(home, &world)?;
            let pack = datapacks::list(&world)
                .into_iter()
                .find(|p| p.file == file)
                .ok_or_else(|| tr("This pack changed. Check the refreshed list."))?;
            datapacks::remove(&world, &pack)
        })();
        self.ui.mods.notice = result.err().map(Err);
        self.world_packs.reread();
    }

    /// Packs › Installed › Data packs: each world with what is in it.
    pub(super) fn installed_data_packs(&mut self, cx: &mut Context<Self>) -> Div {
        self.data_list(cx);
        let query = self.ui.mods.have.text().trim().to_lowercase();
        let editable =
            !self.location_unavailable && self.mods_locked().is_none() && !self.mutating();
        let d = &self.ui.mods.data;
        let mut worlds: Vec<(World, Vec<Listed>)> =
            d.list.as_ref().map_or_else(Vec::new, |l| l.worlds.clone());
        // The world Home asked for leads; the rest keep the order Home has them in.
        worlds.sort_by_key(|(w, _)| Some(&w.path) != d.first.as_ref());
        let confirm = d.confirm.clone();
        let several = worlds.len() > 1;
        let mut groups = div()
            .id("alt-data-list")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&crate::frame::scrolls("data-packs"))
            .flex()
            .flex_col()
            .gap(px(12.));
        let mut shown = 0;
        for (n, (world, packs)) in worlds.iter().enumerate() {
            let whole = query.is_empty() || world.name.to_lowercase().contains(&query);
            let rows: Vec<&Listed> = packs
                .iter()
                .filter(|l| {
                    whole
                        || l.title().to_lowercase().contains(&query)
                        || l.pack.file.to_lowercase().contains(&query)
                })
                .collect();
            if !whole && rows.is_empty() {
                continue;
            }
            shown += 1;
            let count = match packs.len() {
                0 => tr("No data packs yet").to_string(),
                1 => tr("1 data pack").to_string(),
                n => trf("{count} data packs", &[("count", &n.to_string())]),
            };
            let path = world.path.clone();
            let head = div()
                .flex()
                .items_center()
                .gap(px(12.))
                .px(px(16.))
                .py(px(12.))
                .child(world_icon(world, 36.))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .truncate()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(world.name.clone()),
                        )
                        .child(muted(format!("{} · {count}", world_facts(world))).truncate()),
                )
                .child(
                    button(
                        ElementId::named_usize("alt-data-find", n),
                        tr("Find data packs"),
                    )
                    .on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| {
                            this.find_data_packs(Some(path.clone()));
                            cx.notify();
                        },
                    )),
                );
            let mut group = card().flex_none().flex().flex_col().child(head);
            for (i, listed) in rows.into_iter().enumerate() {
                let id = n * 1000 + i;
                let key = (world.path.clone(), listed.pack.file.clone());
                let asked = confirm.as_ref() == Some(&key);
                let remove = button(
                    ElementId::named_usize("alt-data-remove", id),
                    if asked {
                        tr("Move to trash?")
                    } else {
                        tr("Remove")
                    },
                );
                let remove = if editable {
                    remove.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.data_remove(key.0.clone(), key.1.clone());
                        cx.notify();
                    }))
                } else {
                    dim(remove)
                };
                // A folder pack is the player's own work in progress: only a zip is copied.
                let copy = (several && listed.pack.path.is_file()).then(|| {
                    let b = button(
                        ElementId::named_usize("alt-data-copy", id),
                        tr("Add to another world…"),
                    );
                    if !editable {
                        return dim(b);
                    }
                    let what = What::Copy {
                        from: world.path.clone(),
                        title: listed.title(),
                        pack: listed.pack.clone(),
                        origin: listed.origin.clone(),
                    };
                    b.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.pick_worlds(what.clone(), None);
                        cx.notify();
                    }))
                });
                group = group.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .px(px(16.))
                        .py(px(10.))
                        .border_t_1()
                        .border_color(c(palette().divider))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(
                                    div()
                                        .flex()
                                        .items_baseline()
                                        .gap(px(8.))
                                        .child(
                                            div()
                                                .min_w_0()
                                                .truncate()
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .child(listed.title()),
                                        )
                                        .children(
                                            (!listed.pack.enabled)
                                                .then(|| muted(tr("Turned off")).flex_none()),
                                        ),
                                )
                                .when(!listed.pack.description.is_empty(), |d| {
                                    d.child(muted(listed.pack.description.clone()).truncate())
                                }),
                        )
                        .children(copy)
                        .child(remove),
                );
            }
            groups = groups.child(group);
        }
        if worlds.is_empty() {
            groups = groups.child(
                card()
                    .p(px(24.))
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(tr("No worlds yet")),
                    )
                    .child(muted(tr(
                        "A data pack goes into a world. Make one in the game first.",
                    ))),
            );
        } else if shown == 0 {
            groups = groups.child(muted(tr("No packs match your search.")));
        }
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(muted(tr(
                "The game turns a world's data packs on when the world opens, or on /reload.",
            )))
            .children(self.mods_locked().map(muted))
            .child(groups)
    }

    /// The world picker: every world of the setup with a box, and one Add for all ticked.
    pub(crate) fn world_pick_sheet(&mut self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        self.data_list(cx);
        let d = &self.ui.mods.data;
        let pick = d.pick.as_ref()?;
        let worlds: &[(World, Vec<Listed>)] = d.list.as_ref().map_or(&[], |l| &l.worlds);
        let close = |label: &'static str| {
            button("alt-pick-cancel", label).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.close_sheet();
                cx.notify();
            }))
        };
        let heading = crate::sheets::title(trf(
            "Add {pack} to your worlds",
            &[("pack", pick.what.title())],
        ));
        if worlds.is_empty() {
            let body = div()
                .flex()
                .flex_col()
                .child(heading)
                .child(div().text_size(px(tokens::LABEL)).child(crate::selectable::selectable(tr(
                    "A data pack goes into a world, and this profile has none yet. Make one in the game first.",
                ))))
                .child(div().flex().justify_end().child(close(tr("Close"))));
            return Some(crate::sheets::frame("alt-pick", 520., body, cx));
        }
        let mut list = div()
            .flex()
            .flex_col()
            .rounded(crate::tokens::r(crate::tokens::R_CARD))
            .border_1()
            .border_color(c(palette().line))
            .overflow_hidden();
        let mut free = 0;
        for (n, (world, packs)) in worlds.iter().enumerate() {
            let has = pick.what.in_world(world, packs);
            let on = pick.ticked.contains(&world.path);
            free += usize::from(!has);
            let row = div()
                .id(ElementId::named_usize("alt-pick-world", n))
                .flex()
                .items_center()
                .gap(px(12.))
                .px(px(14.))
                .py(px(10.))
                .when(n > 0, |d| d.border_t_1().border_color(c(palette().divider)))
                .child(match has {
                    // Its place is kept so the names line up.
                    true => div().flex_none().size(px(18.)).into_any_element(),
                    false => {
                        // The row is the Tab stop, not the box in it.
                        checkbox(ElementId::named_usize("alt-pick-box", n), on)
                            .tab_stop(false)
                            .into_any_element()
                    }
                })
                .child(world_icon(world, 36.))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .truncate()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(world.name.clone()),
                        )
                        .child(muted(world_facts(world)).truncate()),
                )
                .children(has.then(added_mark));
            list = list.child(if has {
                row
            } else {
                let path = world.path.clone();
                row.reach()
                    .cursor_pointer()
                    .hover(|s| s.bg(c(palette().hover)))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        if let Some(pick) = &mut this.ui.mods.data.pick {
                            if !pick.ticked.remove(&path) {
                                pick.ticked.insert(path.clone());
                            }
                        }
                        cx.notify();
                    }))
            });
        }
        let chosen = pick.ticked.len();
        let label = match chosen {
            0 => tr("Add").to_string(),
            1 => tr("Add to 1 world").to_string(),
            n => trf("Add to {count} worlds", &[("count", &n.to_string())]),
        };
        let add = primary("alt-pick-add", label);
        let add = match chosen {
            0 => dim(add),
            _ => add.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.pick_confirm();
                cx.notify();
            })),
        };
        let say = match free {
            0 => tr("Every world of this profile has it already."),
            _ => tr("Choose the worlds it goes into. The game turns it on when a world opens."),
        };
        let body = div()
            .flex()
            .flex_col()
            .child(heading)
            .child(div().text_size(px(tokens::LABEL)).child(say))
            .child(list)
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(close(tr("Cancel")))
                    .children((free > 0).then_some(add)),
            );
        Some(crate::sheets::frame("alt-pick", 520., body, cx).text_color(c(palette().text)))
    }
}

/// Puts `what` into each of `worlds` of the setup `home`, under the setup's lock, with every
/// world checked again. A project is downloaded (checksum checked) into the first world and
/// that file copied into the rest. Nothing is turned on: the game does that.
fn add_to_worlds(
    what: &What,
    home: &Path,
    worlds: &[World],
    game: &str,
    progress: &dyn Fn(f32),
) -> Result<String, String> {
    let _lock = browser::try_lock_home(home).ok_or_else(|| {
        tr("This profile is busy. Try again when the operation finishes.").to_string()
    })?;
    // The file the next world gets, the note that goes with it, and what the download said.
    let (mut file, mut origin, mut coverage) = match what {
        What::Project(_) => (None, None, None),
        What::Copy { pack, origin, .. } => (Some(pack.path.clone()), origin.clone(), None),
    };
    let (mut added, mut notes, mut failed) = (Vec::new(), Vec::new(), Vec::new());
    for world in worlds {
        let put = check_world(home, &world.path).and_then(|_| match (&file, what) {
            (Some(file), _) => datapacks::add(&world.path, file),
            (None, What::Project(project)) => {
                let source = browser::source(project.source);
                let done = browser::installed::install_datapack(
                    &*source,
                    project,
                    false,
                    &world.path,
                    game,
                    progress,
                )?;
                origin = Some(Origin {
                    world: String::new(),
                    file: String::new(),
                    source: project.source,
                    project: project.id.clone(),
                    title: project.title.clone(),
                    number: done.number,
                });
                coverage = Some(done.coverage.summary());
                Ok(done.pack)
            }
            (None, What::Copy { .. }) => unreachable!("a copy starts with its file"),
        });
        match put {
            Ok(pack) => {
                notes.extend(origin.clone().map(|o| Origin {
                    world: world.folder.clone(),
                    file: pack.file.clone(),
                    ..o
                }));
                file.get_or_insert(pack.path);
                added.push(world.name.clone());
            }
            Err(why) => failed.push(format!("{}: {why}", world.name)),
        }
    }
    if !notes.is_empty() {
        note_origins(home, notes)?;
    }
    let title = match (what, &origin) {
        (What::Project(p), Some(o)) => format!("{} {}", p.title, o.number),
        _ => what.title().to_owned(),
    };
    let said = (!added.is_empty()).then(|| {
        let mut said = trf(
            "Added {pack} to {worlds}. The game turns it on when the world opens.",
            &[("pack", &title), ("worlds", &names(&added))],
        );
        if let Some(coverage) = &coverage {
            said.push(' ');
            said.push_str(coverage);
        }
        said
    });
    match (said, failed.is_empty()) {
        (Some(said), true) => Ok(said),
        (said, _) => Err(said
            .into_iter()
            .chain(failed)
            .collect::<Vec<_>>()
            .join("\n")),
    }
}

#[cfg(test)]
mod tests;
