//! "Another launcher…" under Start from: the profiles other launchers keep, found on this
//! computer (or in a folder the player points at), and what of one to bring into the new
//! profile. Creating it runs in the background, with its progress and Stop in the status strip;
//! a stop leaves no profile behind.
use super::copy_box::{is_on, slot};
use super::*;
use crate::widgets::button::{link, secondary};
use crate::widgets::check::{checkbox, Tick};
use crate::widgets::focus::Reach;
use gpui::AnimationExt as _;
use instances::{CopyOptions, Foreign, ImportProgress};
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};
use sunrise_launcher::native_mods::Cancellation;

/// The options an import can offer, in `copy_box::slot` order: play time, worlds, options, resource
/// packs, screenshots, shader packs, servers. Mods are never among them.
const LABELS: [&str; 7] = [
    i18n::tr_key("Keep play time"),
    i18n::tr_key("Import worlds"),
    i18n::tr_key("Import game options and keys"),
    i18n::tr_key("Import resource packs"),
    i18n::tr_key("Import screenshots"),
    i18n::tr_key("Import shader packs"),
    i18n::tr_key("Import servers"),
];

/// The most rows the list shows before it scrolls.
const LIST_HEIGHT: f32 = 176.;

/// The dialog's state while it starts from another launcher.
#[derive(Default)]
pub(crate) struct ImportState {
    /// What was found: None while it is still being looked for.
    pub(crate) found: Option<Vec<Foreign>>,
    /// The profile chosen, as an index into `found`.
    pub(crate) picked: Option<usize>,
    /// What of it to bring.
    pub(crate) options: CopyOptions,
    /// The name the pick put in the name field: a name the player typed over it is kept.
    prefilled: Option<String>,
}

impl ImportState {
    /// The profile chosen.
    pub(crate) fn chosen(&self) -> Option<&Foreign> {
        self.found.as_ref()?.get(self.picked?)
    }
}

/// The import that is running, as the status strip shows it.
pub(crate) struct Importing {
    pub(crate) name: String,
    pub(crate) cancel: Cancellation,
    pub(crate) progress: Arc<ImportProgress>,
}

/// A profile's muted line: its game version, its worlds and when it was last played.
fn details(f: &Foreign) -> String {
    let mut parts: Vec<String> = Vec::new();
    parts.extend(f.version.clone());
    parts.push(match f.contents.worlds {
        1 => tr("1 world").to_owned(),
        n => trf("{count} worlds", &[("count", &n.to_string())]),
    });
    if let Some(secs) = f.last_played {
        let when = UNIX_EPOCH + Duration::from_secs(secs);
        parts.push(trf(
            "Played {when}",
            &[("when", &crate::worlds_page::ago(when))],
        ));
    }
    parts.join(" · ")
}

/// What the count beside option `n` says.
fn count_of(f: &Foreign, n: usize) -> String {
    let c = &f.contents;
    match n {
        0 => f
            .played_secs
            .map(|s| instances::play_time(Duration::from_secs(s)))
            .unwrap_or_default(),
        1 => c.worlds.to_string(),
        3 => c.resource_packs.to_string(),
        4 => c.screenshots.to_string(),
        5 => c.shader_packs.to_string(),
        _ => String::new(),
    }
}

impl Launcher {
    /// Start from another launcher: the list is looked for in the background.
    pub(super) fn import_start(&mut self, cx: &mut Context<Self>) {
        self.servers.text(Field::ImportSearch).set(String::new());
        let Some(d) = self.instances.dialog.as_mut().filter(|d| !d.busy) else {
            return;
        };
        d.copy = None;
        d.error = None;
        d.menu = None;
        d.import = Some(ImportState::default());
        let found = cx
            .background_executor()
            .spawn(async { instances::import_sources() });
        cx.spawn(async move |this, cx| {
            let found = found.await;
            let _ = this.update(cx, |this, cx| {
                let state = this
                    .instances
                    .dialog
                    .as_mut()
                    .and_then(|d| d.import.as_mut());
                if let Some(state) = state.filter(|s| s.found.is_none()) {
                    state.found = Some(found);
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Picks the profile at `index` of the list: its things are offered, all ticked, and the name
    /// field is filled with its name unless the player typed another.
    fn import_pick(&mut self, index: usize) {
        let Some(state) = self
            .instances
            .dialog
            .as_mut()
            .and_then(|d| d.import.as_mut())
        else {
            return;
        };
        let Some(from) = state.found.as_ref().and_then(|f| f.get(index)).cloned() else {
            return;
        };
        state.picked = Some(index);
        state.options = from.offered();
        let previous = state.prefilled.clone();
        let typed = self.servers.value(Field::InstanceName).trim().to_owned();
        if typed.is_empty() || previous.as_deref() == Some(typed.as_str()) {
            self.servers
                .text(Field::InstanceName)
                .set(from.name.clone());
            if let Some(state) = self
                .instances
                .dialog
                .as_mut()
                .and_then(|d| d.import.as_mut())
            {
                state.prefilled = Some(from.name);
            }
        }
        if let Some(d) = self.instances.dialog.as_mut() {
            d.error = None;
        }
    }

    /// Back from a pick to the list.
    fn import_unpick(&mut self) {
        if let Some(state) = self
            .instances
            .dialog
            .as_mut()
            .and_then(|d| d.import.as_mut())
        {
            state.picked = None;
        }
    }

    /// Asks for a folder and reads it like a launcher's own profile; it joins the list and is
    /// picked.
    fn import_find_folder(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(tr("Choose a game folder").into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let read = cx
                .background_executor()
                .spawn(async move { instances::read_import_folder(&path) })
                .await;
            let _ = this.update(cx, |this, cx| {
                match read {
                    Ok(from) => this.import_add(from),
                    Err(error) => {
                        if let Some(d) = this.instances.dialog.as_mut() {
                            d.error = Some(error);
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Adds a profile read from a folder to the list (once) and picks it.
    fn import_add(&mut self, from: Foreign) {
        let Some(state) = self
            .instances
            .dialog
            .as_mut()
            .and_then(|d| d.import.as_mut())
        else {
            return;
        };
        let found = state.found.get_or_insert_with(Vec::new);
        let index = match found.iter().position(|f| f.folder == from.folder) {
            Some(index) => index,
            None => {
                found.push(from);
                found.len() - 1
            }
        };
        self.import_pick(index);
    }

    /// The dialog body while a launcher is the start: its list, or the pick's box.
    pub(super) fn import_body(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let state = self.instances.dialog.as_ref()?.import.as_ref()?;
        let chosen = state.chosen().cloned();
        let options = state.options;
        let found = state.found.clone();
        Some(match chosen {
            Some(from) => self.import_box(&from, options, cx),
            None => self.import_list(found, cx),
        })
    }

    /// The search over the profiles found, a button for a folder by hand, and the profiles by
    /// launcher.
    fn import_list(&mut self, found: Option<Vec<Foreign>>, cx: &mut Context<Self>) -> AnyElement {
        let p = palette();
        let query = self
            .servers
            .value(Field::ImportSearch)
            .trim()
            .to_lowercase();
        let search = self.field(Field::ImportSearch, tr("Search other launchers"), 300., cx);
        let top = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(div().flex_1().min_w_0().child(search.w_full().min_w_0()))
            .child(
                secondary("new-import-folder", tr("Find another folder…")).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.import_find_folder(cx);
                        cx.notify();
                    },
                )),
            );
        let mut list = div()
            .id("new-import-list")
            .flex()
            .flex_col()
            .gap(px(1.))
            .max_h(px(LIST_HEIGHT))
            .overflow_y_scroll();
        let muted = |text: &str| {
            div()
                .py(px(10.))
                .text_size(px(tokens::LABEL))
                .text_color(rgb(p.muted))
                .child(text.to_owned())
        };
        match &found {
            None => list = list.child(muted(tr("Looking for other launchers…"))),
            Some(all) if all.is_empty() => {
                list = list.child(muted(tr("No other launchers found on this computer.")))
            }
            Some(all) => {
                let mut last: Option<&str> = None;
                let mut shown = 0;
                for (n, f) in all.iter().enumerate() {
                    let hay = format!(
                        "{} {} {}",
                        f.name,
                        f.launcher,
                        f.version.as_deref().unwrap_or("")
                    );
                    if !query.is_empty() && !hay.to_lowercase().contains(&query) {
                        continue;
                    }
                    shown += 1;
                    if last != Some(f.launcher.as_str()) {
                        last = Some(f.launcher.as_str());
                        list = list.child(
                            div()
                                .pt(px(8.))
                                .pb(px(2.))
                                .px(px(8.))
                                .text_size(px(tokens::SMALL))
                                .text_color(rgb(p.muted))
                                .child(f.launcher.clone()),
                        );
                    }
                    list = list.child(self.import_row(n, f, cx));
                }
                if shown == 0 {
                    list = list.child(muted(tr("No profile matches your search.")));
                }
            }
        }
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .pt(px(4.))
            .child(top)
            .child(list)
            .into_any_element()
    }

    /// One profile of the list: its launcher's initial, its name, and what it has.
    fn import_row(&self, n: usize, f: &Foreign, cx: &mut Context<Self>) -> Stateful<Div> {
        let p = palette();
        let initial: String = f.launcher.chars().take(1).collect();
        div()
            .id(ElementId::named_usize("new-import-row", n))
            .reach()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(10.))
            .h(px(44.))
            .px(px(8.))
            .rounded(tokens::r(tokens::R_SMALL))
            .cursor_pointer()
            .hover(move |s| s.bg(rgb(p.hover)))
            .child(
                div()
                    .flex_none()
                    .size(px(28.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(tokens::r(tokens::R_SMALL))
                    .bg(rgb(p.hover))
                    .text_size(px(tokens::LABEL))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(initial),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .truncate()
                            .text_size(px(tokens::LABEL))
                            .child(f.name.clone()),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(px(tokens::SMALL))
                            .text_color(rgb(p.muted))
                            .child(details(f)),
                    ),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.import_pick(n);
                cx.notify();
            }))
    }

    /// The ticks of what the pick has, two to a line, each with how many there are.
    fn import_grid(
        &mut self,
        from: &Foreign,
        options: CopyOptions,
        shown: &[usize],
        cx: &mut Context<Self>,
    ) -> Div {
        let p = palette();
        let mut grid = div().flex().flex_col().gap(px(2.));
        for pair in shown.chunks(2) {
            let mut line = div().flex().gap(px(12.));
            for &n in pair {
                let on = is_on(options, n);
                let count = count_of(from, n);
                line = line.child(
                    div()
                        .id(ElementId::named_usize("new-import-option", n))
                        .reach()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .h(px(30.))
                        .px(px(4.))
                        .rounded(tokens::r(tokens::R_SMALL))
                        .cursor_pointer()
                        .hover(|s| s.bg(rgb(p.hover)))
                        .child(
                            checkbox(
                                ElementId::named_usize("new-import-check", n),
                                if on { Tick::On } else { Tick::Off },
                            )
                            .tab_stop(false),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(px(tokens::LABEL))
                                .child(tr(LABELS[n])),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_size(px(tokens::SMALL))
                                .text_color(rgb(p.muted))
                                .child(count),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(state) = this
                                .instances
                                .dialog
                                .as_mut()
                                .filter(|d| !d.busy)
                                .and_then(|d| d.import.as_mut())
                            {
                                let value = slot(&mut state.options, n);
                                *value = !*value;
                            }
                            cx.notify();
                        })),
                );
            }
            if pair.len() == 1 {
                line = line.child(div().flex_1());
            }
            grid = grid.child(line);
        }
        grid
    }

    /// The pick's name and launcher, and the way back to the list.
    fn import_head(&mut self, from: &Foreign, cx: &mut Context<Self>) -> Div {
        let p = palette();
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_baseline()
                    .gap(px(8.))
                    .text_size(px(tokens::LABEL))
                    .child(div().truncate().child(from.name.clone()))
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(tokens::SMALL))
                            .text_color(rgb(p.muted))
                            .child(from.launcher.clone()),
                    ),
            )
            .child(
                link("new-import-change", tr("Choose another")).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.import_unpick();
                        cx.notify();
                    },
                )),
            )
    }

    /// "What to import": the pick, then a tick for each thing it has, with how many, and the
    /// note that mods stay behind.
    fn import_box(
        &mut self,
        from: &Foreign,
        options: CopyOptions,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = palette();
        let offered = from.offered();
        let shown: Vec<usize> = (0..LABELS.len()).filter(|n| is_on(offered, *n)).collect();
        let all_on = shown.iter().all(|n| is_on(options, *n));
        let head = self.import_head(from, cx);
        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .child(
                div()
                    .text_size(px(tokens::SMALL))
                    .text_color(rgb(p.muted))
                    .child(tr("What to import")),
            )
            .child(
                link(
                    "new-import-all",
                    if all_on {
                        tr("Select none")
                    } else {
                        tr("Select all")
                    },
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(state) = this
                        .instances
                        .dialog
                        .as_mut()
                        .filter(|d| !d.busy)
                        .and_then(|d| d.import.as_mut())
                    {
                        let offered = state.chosen().map(Foreign::offered);
                        let all_on = offered.is_some_and(|o| {
                            (0..LABELS.len()).all(|n| !is_on(o, n) || is_on(state.options, n))
                        });
                        state.options = match (all_on, offered) {
                            (false, Some(offered)) => offered,
                            _ => CopyOptions::nothing(),
                        };
                    }
                    cx.notify();
                })),
            );
        let grid = self.import_grid(from, options, &shown, cx);
        let mods = match from.contents.mods {
            0 => tr("Mods aren't imported: other launchers' mods don't run in Sunrise.").to_owned(),
            n => trf(
                "Mods aren't imported: other launchers' mods don't run in Sunrise ({count} left behind).",
                &[("count", &n.to_string())],
            ),
        };
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .pt(px(12.))
            .border_t_1()
            .border_color(rgb(p.divider))
            .child(head)
            .child(header)
            .child(grid)
            .child(
                div()
                    .text_size(px(tokens::SMALL))
                    .text_color(rgb(p.muted))
                    .child(mods),
            )
            .with_animation(
                "new-import-box",
                crate::motion::eased(sunrise_launcher_theme::QUICK),
                |d, t| d.opacity(t),
            )
            .into_any_element()
    }

    /// Create with a launcher as the start: Create profile copies the pick in the background.
    /// Whether the dialog was one (so the ordinary save is not to run).
    pub(crate) fn save_import(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(d) = self
            .instances
            .dialog
            .as_ref()
            .filter(|d| !d.busy && d.editing.is_none() && d.import.is_some())
        else {
            return false;
        };
        let name = self.servers.value(Field::InstanceName).trim().to_owned();
        let group = Some(self.servers.value(Field::InstanceGroup).trim().to_owned())
            .filter(|g| !g.is_empty());
        let state = d.import.as_ref();
        let from = state.and_then(|s| s.chosen()).cloned();
        let options = state.map_or(CopyOptions::nothing(), |s| s.options);
        let error = if self.instances.importing.is_some() {
            Some(tr("Another import is still running."))
        } else if from.is_none() {
            Some(tr("Choose a profile to import."))
        } else if name.is_empty() {
            Some(tr("A profile needs a name."))
        } else {
            None
        };
        let taken = self
            .instances
            .list
            .iter()
            .any(|i| i.name.eq_ignore_ascii_case(&name))
            .then(|| instances::name_taken(&name));
        let Some(d) = self.instances.dialog.as_mut() else {
            return true;
        };
        if let Some(error) = error.map(str::to_owned).or(taken) {
            d.error = Some(error);
            return true;
        }
        let Some(from) = from else {
            return true;
        };
        let (icon, pin, channel) = (d.icon.clone(), d.pin.clone(), d.channel);
        let picture = d
            .picture
            .as_ref()
            .map(|p| p.as_ref().map(|c| c.png.clone()));
        let cancel = Cancellation::default();
        let progress = Arc::new(ImportProgress::default());
        self.instances.importing = Some(Importing {
            name: name.clone(),
            cancel: cancel.clone(),
            progress: progress.clone(),
        });
        // The strip carries it from here: the dialog is done.
        self.instances.dialog = None;
        let called = name.clone();
        let task = cx.background_executor().spawn(async move {
            instances::create_importing(
                &called,
                &from,
                options,
                &cancel,
                &progress,
                picture.as_ref().map(Option::as_deref),
                |m: &mut instances::Meta| {
                    m.name = Some(name);
                    m.group = group;
                    m.icon = icon;
                    m.version = pin;
                    m.channel = Some(channel);
                },
            )
        });
        self.watch_import(task, cx);
        cx.notify();
        true
    }

    /// Follows a running import: the strip's bar moves with the copy, and what it ends with is
    /// said and selected.
    fn watch_import(
        &mut self,
        task: gpui::Task<Result<instances::Instance, String>>,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(150))
                    .await;
                // The strip's bar moves with the copy.
                match this.update(cx, |this, cx| {
                    cx.notify();
                    this.instances.importing.is_some()
                }) {
                    Ok(true) => {}
                    _ => break,
                }
            }
        })
        .detach();
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                let stopped = this
                    .instances
                    .importing
                    .take()
                    .is_some_and(|job| job.cancel.is_stopped());
                match result {
                    Ok(instance) => {
                        crate::toast::say(trf("Created {name}", &[("name", &instance.name)]));
                        this.select_instance(&instance);
                        this.reload_instances();
                        this.refresh_installed();
                        if matches!(
                            instance.kind,
                            instances::Kind::Default | instances::Kind::Own
                        ) {
                            this.navigate(crate::frame::Page::Home);
                        }
                    }
                    Err(_) if stopped => crate::toast::say(tr("The import was stopped.")),
                    Err(error) => crate::toast::say(error),
                }
                cx.notify();
            });
        })
        .detach();
    }
}
