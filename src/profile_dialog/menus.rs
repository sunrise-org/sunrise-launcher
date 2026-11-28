//! The dialog's dropdowns: the game build, and what the profile starts from.
use super::*;
use crate::instances_page::DialogMenu;
use crate::sheets::channel_name;
use crate::widgets::dropdown::{self, Close, Face};
use std::rc::Rc;

/// The small muted label over a field.
pub(super) fn small_label(text: SharedString) -> Div {
    div()
        .text_size(px(tokens::SMALL))
        .text_color(rgb(palette().muted))
        .child(text)
}

/// A field that opens a menu: an optional icon, its text, a muted aside, and a chevron.
fn dropdown_field(
    id: &'static str,
    open: bool,
    glyph: Option<&'static str>,
    text: String,
    aside: Option<String>,
    enabled: bool,
) -> Stateful<Div> {
    let text = SharedString::from(text);
    let face = Face {
        sizers: vec![text.clone()],
        text,
        glyph,
        aside: aside.map(SharedString::from),
    };
    dropdown::trigger(id, open, face, enabled, true)
}

/// One row of a menu: a heading over a muted line, a tick when it is the current choice.
fn menu_row(
    id: impl Into<ElementId>,
    head: String,
    sub: Option<String>,
    on: bool,
) -> Stateful<Div> {
    dropdown::row(id, head, sub.map(SharedString::from), on)
}

/// A menu hung under its field `id` (whose parent is `relative`), over everything after it; a
/// press anywhere else in the window closes it.
fn menu_under(
    id: &'static str,
    rows: Vec<AnyElement>,
    current: Option<usize>,
    cx: &mut Context<Launcher>,
) -> [gpui::Deferred; 2] {
    let count = rows.len();
    dropdown::layer(
        &SharedString::from(id),
        rows,
        count,
        current,
        None,
        dialog_close(cx),
    )
}

/// Closes the dialog's open menu.
fn dialog_close(cx: &mut Context<Launcher>) -> Close {
    let this = cx.entity();
    Rc::new(move |_: &mut Window, cx: &mut App| {
        this.update(cx, |this, cx| {
            // Also the build chooser's open submenu.
            this.close_menus();
            if let Some(d) = this.instances.dialog.as_mut() {
                d.menu = None;
            }
            cx.notify();
        })
    })
}

/// A field's keys: the arrows open its menu, Escape closes it.
fn with_menu_keys(
    field: Stateful<Div>,
    which: DialogMenu,
    open: bool,
    cx: &mut Context<Launcher>,
) -> Stateful<Div> {
    let this = cx.entity();
    let toggle: Close = Rc::new(move |_: &mut Window, cx: &mut App| {
        this.update(cx, |this, cx| {
            let was = this.instances.dialog.as_ref().and_then(|d| d.menu);
            // One menu at a time, and an open build chooser's submenu goes with it.
            this.close_menus();
            if let Some(d) = this.instances.dialog.as_mut().filter(|d| !d.busy) {
                d.menu = (was != Some(which)).then_some(which);
            }
            cx.notify();
        })
    });
    dropdown::with_keys(field, open, toggle, dialog_close(cx))
}

impl Launcher {
    /// "Game build": one dropdown for the channel it follows or the version it keeps.
    pub(super) fn profile_build_row(&mut self, cx: &mut Context<Self>) -> Div {
        let Some(d) = self.instances.dialog.as_ref() else {
            return div();
        };
        let (channel, pin, open) = (d.channel, d.pin.clone(), d.menu == Some(DialogMenu::Build));
        let bound = d
            .copy
            .as_ref()
            .and_then(instances::CopyPreview::base)
            .map(|tag| self.version_label(tag));
        let p = palette();
        let follows = |channel: Channel| {
            trf(
                "{channel} · follow latest",
                &[("channel", channel_name(channel))],
            )
        };
        let current = match (&bound, &pin) {
            (Some(label), _) => label.clone(),
            (None, Some(tag)) => self.version_label(tag),
            (None, None) => follows(channel),
        };
        let row = div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(small_label(tr("Game build").into()));
        let field = dropdown_field(
            "new-build",
            open,
            Some("package-01"),
            current,
            None,
            bound.is_none(),
        );
        if let Some(build) = bound.as_deref() {
            return row.child(field).child(
                div()
                    .text_size(px(tokens::SMALL))
                    .text_color(rgb(p.muted))
                    .child(trf(
                        "This copy keeps {build} from the selected profile.",
                        &[("build", build)],
                    )),
            );
        }
        let field = with_menu_keys(field, DialogMenu::Build, open, cx);
        // The same chooser as Home's build button (`sheets/build_menu.rs`); picking only sets
        // this dialog's build.
        let mut wrap = div()
            .relative()
            .child(field.on_click(cx.listener(|this, _, _, cx| {
                let was = this
                    .instances
                    .dialog
                    .as_ref()
                    .is_some_and(|d| d.menu == Some(DialogMenu::Build));
                this.close_menus();
                if let Some(d) = this.instances.dialog.as_mut().filter(|d| !d.busy) {
                    d.menu = (!was).then_some(DialogMenu::Build);
                }
                cx.notify();
            })));
        if open {
            wrap = wrap.children(self.dialog_build_menu(dialog_close(cx), cx));
        } else {
            dropdown::forget("new-build");
        }
        let mut row = row.child(wrap);
        if channel == Channel::Stable && !self.stable_available() && pin.is_none() {
            row = row.child(
                div()
                    .text_size(px(tokens::SMALL))
                    .text_color(rgb(p.muted))
                    .child(tr("No Stable build yet.")),
            );
        }
        row
    }

    /// "Start from": an empty profile, or a copy of one that exists.
    pub(super) fn profile_start_row(&mut self, cx: &mut Context<Self>) -> Div {
        let Some(d) = self.instances.dialog.as_ref() else {
            return div();
        };
        let open = d.menu == Some(DialogMenu::Start);
        let source = d.copy.as_ref().map(|c| c.source().id.clone());
        let importing = d.import.is_some();
        let shown = match d.copy.as_ref() {
            Some(copy) => trf("Copy “{name}”", &[("name", &copy.source().name)]),
            None if importing => tr("Another launcher…").to_owned(),
            None => tr("Empty profile").to_owned(),
        };
        let field = dropdown_field("new-start", open, None, shown, None, true).on_click(
            cx.listener(|this, _, _, cx| {
                if let Some(d) = this.instances.dialog.as_mut().filter(|d| !d.busy) {
                    d.menu = (d.menu != Some(DialogMenu::Start)).then_some(DialogMenu::Start);
                }
                cx.notify();
            }),
        );
        let field = with_menu_keys(field, DialogMenu::Start, open, cx);
        let mut wrap = div().relative().child(field);
        if open {
            let mut rows: Vec<AnyElement> = vec![menu_row(
                "new-start-empty",
                tr("Empty profile").into(),
                Some(tr("No mods, packs or worlds").into()),
                source.is_none() && !importing,
            )
            .on_click(cx.listener(|this, _, _, cx| {
                if let Some(d) = this.instances.dialog.as_mut().filter(|d| !d.busy) {
                    d.copy = None;
                    d.import = None;
                    d.error = None;
                    d.menu = None;
                }
                cx.notify();
            }))
            .into_any_element()];
            rows.push(
                menu_row(
                    "new-start-import",
                    tr("Another launcher…").into(),
                    Some(tr("Worlds, options and packs from Prism, Modrinth and others").into()),
                    importing,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.import_start(cx);
                    cx.notify();
                }))
                .into_any_element(),
            );
            let profiles: Vec<Instance> = self
                .instances
                .list
                .iter()
                .filter(|i| i.kind != instances::Kind::Pack)
                .cloned()
                .collect();
            // The row to show on open: Empty profile is first, the copies follow it.
            let current = Some(match source.as_deref() {
                None if importing => 1,
                None => 0,
                Some(id) => 2 + profiles.iter().position(|p| p.id == id).unwrap_or(0),
            });
            for (idx, profile) in profiles.into_iter().enumerate() {
                let sub = match profile.last_played {
                    Some(when) => trf("Played {when}", &[("when", &crate::worlds_page::ago(when))]),
                    None => tr("Never played").to_owned(),
                };
                let lock = self.copy_lock_for(&profile);
                let row = menu_row(
                    ElementId::named_usize("new-start-copy", idx),
                    trf("Copy “{name}”", &[("name", &profile.name)]),
                    Some(lock.map_or(sub, str::to_owned)),
                    source.as_deref() == Some(profile.id.as_str()),
                );
                rows.push(
                    (match lock {
                        Some(_) => row.opacity(0.45).cursor_default().tab_stop(false),
                        None => row.on_click(cx.listener(move |this, _, _, cx| {
                            let preview = instances::CopyPreview::read(&profile);
                            if let Some(d) = this.instances.dialog.as_mut().filter(|d| !d.busy) {
                                match preview {
                                    Ok(copy) => {
                                        d.copy = Some(copy);
                                        d.import = None;
                                        d.error = None;
                                    }
                                    Err(error) => d.error = Some(error),
                                }
                                d.menu = None;
                            }
                            cx.notify();
                        })),
                    })
                    .into_any_element(),
                );
            }
            wrap = wrap.children(menu_under("new-start", rows, current, cx));
        } else {
            dropdown::forget("new-start");
        }
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(small_label(tr("Start from").into()))
            .child(wrap)
    }
}
