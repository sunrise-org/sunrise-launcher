//! The selected setup’s local collections. Reading a list never creates its folder. Choosing a
//! file targets the collection that opened the dialog, even if another setup exists.
use super::selection::{bar, Gesture, PackOp};
use super::ui::{c, card, dim, muted};
use super::*;
use crate::paint::palette;
use crate::widgets::button::{primary, secondary};
use crate::widgets::check::{checkbox, switch, Tick};
use crate::widgets::feedback::empty_state_icon;
use gpui::ExternalPaths;
use gpui::SharedString;
use std::path::Path;
use sunrise_launcher::packs::{self, Kind};

impl Launcher {
    pub(crate) fn reset_pack_view(&mut self) {
        let m = &mut self.mods_state;
        m.have.set(String::new());
        m.row_filter.text.clear();
        m.selection.clear();
        m.anchor = None;
        m.pack_selection.clear();
        self.dismiss_notice();
        self.game_packs.confirm_remove = None;
        self.game_packs.stale();
    }

    pub(super) fn pack_page(&mut self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let focus = self
            .mods_state
            .have_focus
            .get_or_insert_with(|| cx.focus_handle().tab_index(0).tab_stop(true))
            .clone();
        let header = self.mods_header(&focus, window, cx);
        let kind = self.mods_state.shelf.kind();
        let home = self.mods_home();
        let list = match kind {
            Some(kind) => self.installed_packs(kind, cx),
            None => self.installed_data_packs(cx),
        };
        div()
            .id("alt-packs-page")
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .gap(px(14.))
            .p(px(24.))
            .text_color(c(palette().text))
            .child(header)
            .children(self.strip(cx))
            .child(list)
            // A file dropped on the page joins the collection shown; a data pack needs a world.
            .when_some(kind, |page, kind| {
                page.drag_over::<ExternalPaths>(|s, _, _, _| {
                    s.border_1().border_color(c(palette().muted))
                })
                .on_drop(cx.listener(
                    move |this, paths: &ExternalPaths, _, cx| {
                        this.import_pack_files(home.clone(), kind, paths.paths().to_vec(), cx);
                        cx.stop_propagation();
                    },
                ))
            })
            .into_any_element()
    }

    /// The pack kinds as the shared underline tabs.
    pub(super) fn collection_tabs(&self, cx: &mut Context<Self>) -> Div {
        let chosen = self.mods_state.shelf;
        let tabs: Vec<(Shelf, SharedString)> = [
            (Shelf::Resource, tr("Resource packs")),
            (Shelf::Shader, tr("Shader packs")),
            (Shelf::Data, tr("Data packs")),
        ]
        .into_iter()
        .map(|(kind, label)| (kind, label.into()))
        .collect();
        let this = cx.entity();
        crate::widgets::nav::tab_row(
            crate::widgets::nav::tabs("alt-collection-tabs", &tabs, chosen, move |kind, _, cx| {
                this.update(cx, |this, cx| {
                    if this.mods_state.shelf != kind {
                        this.mods_state.shelf = kind;
                        this.reset_pack_view();
                        this.worlds.stale();
                        this.mods_state.data.reread();
                    }
                    cx.notify();
                })
            })
            .seen(),
            None,
        )
    }

    fn can_edit_packs(&self, home: &Path, kind: Kind) -> bool {
        !self.location_unavailable
            && self.mods_locked().is_none()
            && !self.mutating()
            && self.ui.page == Page::Packs
            && self.mods_state.shelf.kind() == Some(kind)
            && self.selected_instance().is_some_and(|i| i.home == home)
    }

    fn import_pack(&mut self, home: PathBuf, kind: Kind, cx: &mut Context<Self>) {
        if !self.can_edit_packs(&home, kind) {
            return;
        }
        let pick = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some(tr("Import from file").into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = pick.await {
                let _ = this.update(cx, |this, cx| this.import_pack_files(home, kind, paths, cx));
            }
        })
        .detach();
    }

    fn import_pack_files(
        &mut self,
        home: PathBuf,
        kind: Kind,
        paths: Vec<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        // A dialog or drop may outlive the view that offered it.
        if paths.is_empty() || !self.can_edit_packs(&home, kind) {
            return;
        }
        self.dismiss_notice();
        self.mods_state.busy = Some((tr("Importing packs…").into(), 0.));
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let _lock =
                        sunrise_launcher::browser::try_lock_home(&home).ok_or_else(|| {
                            tr("This profile is busy. Try again when the operation finishes.")
                                .to_string()
                        })?;
                    let mut added = Vec::new();
                    let mut refused = Vec::new();
                    for path in paths {
                        match packs::add_local(&home, kind, &path) {
                            Ok(p) => added.push(p.title),
                            Err(e) => refused.push(e),
                        }
                    }
                    let mut parts = Vec::new();
                    if !added.is_empty() {
                        parts.push(trf("Imported {name}.", &[("name", &added.join(", "))]));
                    }
                    if refused.is_empty() {
                        Ok(parts.join(" "))
                    } else {
                        parts.extend(refused);
                        Err(parts.join("\n"))
                    }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.mods_state.busy = None;
                this.mods_state.notice = Some(result);
                this.game_packs.stale();
                cx.notify();
            });
        })
        .detach();
    }

    fn change_pack(
        &mut self,
        home: &Path,
        kind: Kind,
        change: impl FnOnce() -> Result<(), String>,
    ) {
        if !self.can_edit_packs(home, kind) {
            return;
        }
        let result = match sunrise_launcher::browser::try_lock_home(home) {
            Some(_guard) => change(),
            None => Err(tr("This profile is busy. Try again when the operation finishes.").into()),
        };
        self.mods_state.notice = result.err().map(Err);
        self.game_packs.confirm_remove = None;
        self.game_packs.stale();
    }

    pub(super) fn installed_packs(&mut self, kind: Kind, cx: &mut Context<Self>) -> Div {
        let Some(instance) = self.selected_instance() else {
            return div();
        };
        let home = instance.home.clone();
        self.load_packs_list(home.clone());
        let editable = self.can_edit_packs(&home, kind);
        let query = self.mods_state.have.text().trim().to_lowercase();
        let (detail, empty) = match kind {
            Kind::Resource => (
                tr("Higher packs take priority. A pack you turn on goes to the top."),
                tr("Or drop a .zip on this page."),
            ),
            Kind::Shader => (
                tr("Choose one shader pack at a time, or leave them all off."),
                tr("Or drop a .zip or .sshader on this page."),
            ),
        };
        let add = secondary("alt-pack-import", tr("Import from file…"));
        let add = if editable {
            let dest = home.clone();
            add.on_click(
                cx.listener(move |this, _, _, cx| this.import_pack(dest.clone(), kind, cx)),
            )
        } else {
            dim(add)
        };
        let folder = home.join(kind.folder());
        let open = secondary("alt-pack-folder", tr("Open folder"));
        let open = if folder.is_dir() || editable {
            open.on_click(cx.listener(move |this, _, _, cx| {
                match std::fs::create_dir_all(&folder) {
                    Ok(()) => crate::toast::open(cx, &folder),
                    Err(e) => this.mods_state.notice = Some(Err(e.to_string())),
                }
                cx.notify();
            }))
        } else {
            dim(open)
        };
        let header = div()
            .flex()
            .items_center()
            .gap(px(14.))
            .child(div().flex_1().min_w_0().child(muted(detail)))
            .child(open.flex_none())
            .child(add.flex_none());
        let items: Vec<_> = match kind {
            Kind::Resource => self
                .game_packs
                .resource
                .iter()
                .map(|(p, icon)| (p, icon.clone()))
                .collect(),
            Kind::Shader => self.game_packs.shader.iter().map(|p| (p, None)).collect(),
        };
        // Rows already gone drop out of the ticks; the rest keep them as the search narrows.
        let everything: Vec<String> = items.iter().map(|(pack, _)| pack.file.clone()).collect();
        self.mods_state.pack_selection.retain(&everything);
        let order = std::rc::Rc::new(self.shown_pack_files(kind));
        let ticked = self.mods_state.pack_selection.clone();
        let mut list = card()
            .id("alt-collection-list")
            .min_h_0()
            .overflow_y_scroll()
            .pr(px(12.))
            .flex()
            .flex_col();
        let mut shown = 0;
        for (index, (pack, icon)) in items.iter().enumerate() {
            if !query.is_empty()
                && !pack.title.to_lowercase().contains(&query)
                && !pack.file.to_lowercase().contains(&query)
            {
                continue;
            }
            let toggle = switch(
                ElementId::named_usize("alt-pack-switch", index),
                pack.enabled,
            );
            let toggle = if editable {
                let (dest, file, was_on) = (home.clone(), pack.file.clone(), pack.enabled);
                toggle.on_click(cx.listener(move |this, _, _, cx| {
                    this.change_pack(&dest, kind, || {
                        let p = packs::find(&dest, kind, &file)?;
                        if p.file != file || p.enabled != was_on {
                            return Err(tr("This pack changed. Check the refreshed list.").into());
                        }
                        packs::set_enabled(&dest, &p, !was_on)
                    });
                    cx.notify();
                }))
            } else {
                dim(toggle)
            };
            // Fixed columns, as the installed mods list has them: the switch, the picture,
            // the name, then the row's actions.
            let is_ticked = ticked.contains(&pack.file);
            // Its box shows under the pointer, and on every row once one is ticked.
            let check = {
                let (order, file) = (order.clone(), pack.file.clone());
                div()
                    .flex_none()
                    .opacity(if ticked.is_empty() { 0. } else { 1. })
                    .group_hover("alt-pack-row", |s| s.opacity(1.))
                    .child(
                        checkbox(
                            ElementId::named_usize("alt-pack-check", index),
                            if is_ticked { Tick::On } else { Tick::Off },
                        )
                        .on_click(cx.listener(
                            move |this, ev: &ClickEvent, _, cx| {
                                cx.stop_propagation();
                                this.mods_state.pack_selection.click(
                                    &order,
                                    &file,
                                    ev.modifiers().shift,
                                );
                                cx.notify();
                            },
                        )),
                    )
            };
            let mut row = div()
                .id(ElementId::named_usize("alt-pack-row", index))
                .group("alt-pack-row")
                .flex_none()
                .min_h(px(52.))
                .flex()
                .items_center()
                .gap(px(12.))
                .px(px(14.))
                .when(is_ticked, |d| d.bg(c(palette().selected)))
                .child(check)
                .child(toggle)
                .when(shown > 0, |d| d.border_t_1().border_color(palette().rule()))
                // The pack another page opened the shelf at.
                .when(
                    self.mods_state
                        .reveal
                        .as_ref()
                        .is_some_and(|r| r.is_pack(pack)),
                    |d| {
                        d.bg(c(palette().hover)).child(super::ui::reveal(
                            crate::frame::scrolls("packs"),
                            self.mods_state.revealed.clone(),
                            None,
                        ))
                    },
                );
            shown += 1;
            row = row
                .child(crate::mod_icons::tile(
                    icon.clone(),
                    &pack.file,
                    &pack.title,
                    32.,
                ))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .truncate()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(pack.title.clone()),
                        )
                        .when(!pack.description.is_empty(), |d| {
                            d.child(muted(tr(&pack.description).to_owned()).truncate())
                        }),
                );
            if kind == Kind::Resource && pack.enabled && query.is_empty() {
                for (label, adjacent) in [
                    (tr("Move up"), index.checked_sub(1)),
                    (tr("Move down"), Some(index + 1)),
                ] {
                    let Some((neighbor, _)) = adjacent
                        .and_then(|i| items.get(i))
                        .filter(|(p, _)| p.enabled)
                    else {
                        continue;
                    };
                    let b = secondary(
                        SharedString::from(format!("alt-order-{index}-{adjacent:?}")),
                        label,
                    );
                    let (file, neighbor, dest) =
                        (pack.file.clone(), neighbor.file.clone(), home.clone());
                    row = row.child(if editable {
                        b.on_click(cx.listener(move |this, _, _, cx| {
                            this.change_pack(&dest, kind, || {
                                packs::move_resource(&dest, &file, &neighbor)
                            });
                            cx.notify();
                        }))
                    } else {
                        dim(b)
                    });
                }
            }
            let key = (kind, pack.file.clone());
            let confirm = self.game_packs.confirm_remove.as_ref() == Some(&key);
            let remove = secondary(
                format!("alt-pack-remove-{}", index),
                if confirm {
                    tr("Move to trash?")
                } else {
                    tr("Remove")
                },
            );
            let remove = if editable {
                let dest = home.clone();
                remove.on_click(cx.listener(move |this, _, _, cx| {
                    if !this.can_edit_packs(&dest, kind) {
                        return;
                    }
                    if this.game_packs.confirm_remove.as_ref() == Some(&key) {
                        this.change_pack(&dest, kind, || {
                            let p = packs::find(&dest, kind, &key.1)?;
                            if p.file != key.1 {
                                return Err(
                                    tr("This pack changed. Check the refreshed list.").into()
                                );
                            }
                            packs::remove(&dest, &p)?;
                            // Otherwise Discover thinks this version is still installed and
                            // will not download it again. change_pack holds the home lock.
                            let mut kept = installed::Installed::load(&dest);
                            let before = kept.entries.len();
                            kept.entries
                                .retain(|entry| entry.path(&dest).as_deref() != Some(&p.path));
                            if kept.entries.len() != before {
                                kept.save(&dest)?;
                            }
                            Ok(())
                        });
                    } else {
                        this.game_packs.confirm_remove = Some(key.clone());
                    }
                    cx.notify();
                }))
            } else {
                dim(remove)
            };
            let (order, file) = (order.clone(), pack.file.clone());
            list = list.child(row.child(remove).on_click(cx.listener(
                move |this, ev: &ClickEvent, _, cx| {
                    // Ctrl (Cmd) or Shift ticks the row; a plain press on it does nothing.
                    let gesture = Gesture::of(&ev.modifiers());
                    if gesture.selects() {
                        this.mods_state
                            .pack_selection
                            .click(&order, &file, gesture.range);
                        cx.notify();
                    }
                },
            )));
        }
        // None at all: the empty state stands in the list's place, centred in the page.
        let none = items.is_empty().then(|| {
            let (none, find) = match kind {
                Kind::Resource => (tr("No resource packs yet"), tr("Find resource packs")),
                Kind::Shader => (tr("No shader packs yet"), tr("Find shader packs")),
            };
            empty_state_icon(
                Some("package-01"),
                none,
                empty,
                Some(
                    primary("alt-packs-browse", find)
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.choose_view(Page::PackFind);
                            cx.notify();
                        }))
                        .into_any_element(),
                ),
            )
        });
        if none.is_none() && shown == 0 {
            list = list.child(muted(tr("No packs match your search.")).p(px(20.)));
        }
        let count = |on: bool| {
            items
                .iter()
                .filter(|(pack, _)| pack.enabled == on && ticked.contains(&pack.file))
                .count()
        };
        let pick_bar = self.pack_bar(
            kind,
            &home,
            (count(true), count(false)),
            &ticked,
            &order,
            cx,
        );
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(header)
            .children(self.mods_locked().map(muted))
            .map(|d| match none {
                Some(none) => d.child(none),
                None => d.child(crate::scrollbar::Scrollbar::page("packs").wrap(list, &self.theme)),
            })
            .children(pick_bar)
    }

    /// The bar under the shelf while packs are ticked: enable, disable and remove, each one
    /// change for all of them. Only what applies is offered.
    fn pack_bar(
        &self,
        kind: Kind,
        home: &Path,
        (on, off): (usize, usize),
        ticked: &super::selection::Selection,
        order: &std::rc::Rc<Vec<String>>,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if ticked.is_empty() {
            return None;
        }
        let toggle = {
            let order = order.clone();
            bar::link(
                "alt-pack-pick-all",
                if ticked.all_of(order.as_slice()) {
                    tr("Clear")
                } else {
                    tr("Select all")
                },
            )
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.mods_state.pack_selection.toggle_all(&order);
                cx.notify();
            }))
        };
        let mut actions: Vec<AnyElement> = Vec::new();
        let mut push = |id: &'static str, label: String, op: PackOp| {
            let home = home.to_owned();
            actions.push(
                bar::action(id, label, true, op == PackOp::Remove)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.pack_bar_run(home.clone(), kind, op, cx);
                        cx.notify();
                    }))
                    .into_any_element(),
            );
        };
        // One shader pack is on at a time: turning several on has no meaning.
        if off > 0 && (kind == Kind::Resource || ticked.len() == 1) {
            push("alt-pack-pick-on", tr("Enable").into(), PackOp::Enable);
        }
        if on > 0 {
            push("alt-pack-pick-off", tr("Disable").into(), PackOp::Disable);
        }
        let n = ticked.len().to_string();
        push(
            "alt-pack-pick-remove",
            if ticked.removal_armed() {
                trf("Confirm: remove {count}", &[("count", &n)])
            } else {
                trf("Remove {count}…", &[("count", &n)])
            },
            PackOp::Remove,
        );
        Some(bar::bar(ticked.len(), toggle, actions))
    }

    /// Does `op` to the ticked packs. Remove asks once first: the same press on the same ticks
    /// confirms it.
    fn pack_bar_run(&mut self, home: PathBuf, kind: Kind, op: PackOp, cx: &mut Context<Self>) {
        if op == PackOp::Remove && !self.mods_state.pack_selection.confirm_removal() {
            return;
        }
        let files: Vec<String> = self
            .mods_state
            .pack_selection
            .keys()
            .iter()
            .cloned()
            .collect();
        if files.is_empty() {
            return;
        }
        self.run_pack_op(home, kind, files, op, cx);
        self.mods_state.pack_selection.clear();
    }
}
