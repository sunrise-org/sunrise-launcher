//! The dialog's dropdowns: the game build, and what the profile starts from.
use super::*;
use crate::instances_page::DialogMenu;
use crate::sheets::channel_name;
use crate::widgets::dropdown::{self, Close, Face};
use crate::widgets::focus::Reach;
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
    cx: &mut Context<Launcher>,
) -> [gpui::Deferred; 2] {
    let count = rows.len();
    dropdown::layer(&SharedString::from(id), rows, count, None, dialog_close(cx))
}

/// Closes the dialog's open menu.
fn dialog_close(cx: &mut Context<Launcher>) -> Close {
    let this = cx.entity();
    Rc::new(move |_: &mut Window, cx: &mut App| {
        this.update(cx, |this, cx| {
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
            if let Some(d) = this.instances.dialog.as_mut().filter(|d| !d.busy) {
                d.menu = (d.menu != Some(which)).then_some(which);
                d.pinning = false;
            }
            cx.notify();
        })
    });
    dropdown::with_keys(field, open, toggle, dialog_close(cx))
}

fn divider() -> AnyElement {
    div()
        .h(px(1.))
        .my(px(3.))
        .bg(rgb(palette().divider))
        .into_any_element()
}

fn describe(channel: Channel) -> &'static str {
    match channel {
        Channel::Stable => tr("Tested releases, updated less often"),
        Channel::Nightly => tr("The newest build, every night"),
        Channel::Dev => tr("Builds of work in progress"),
    }
}

impl Launcher {
    /// "Game build": one dropdown for the channel it follows or the version it keeps.
    pub(super) fn profile_build_row(&mut self, cx: &mut Context<Self>) -> Div {
        let Some(d) = self.instances.dialog.as_ref() else {
            return div();
        };
        let (channel, pin, open, pinning) = (
            d.channel,
            d.pin.clone(),
            d.menu == Some(DialogMenu::Build),
            d.pinning,
        );
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
        let target = self.target_version(pin.as_ref(), channel).cloned();
        let age = target.as_deref().and_then(|tag| self.build_age(tag));
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
            age,
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
        let mut wrap = div()
            .relative()
            .child(field.on_click(cx.listener(|this, _, _, cx| {
                if let Some(d) = this.instances.dialog.as_mut().filter(|d| !d.busy) {
                    d.menu = (d.menu != Some(DialogMenu::Build)).then_some(DialogMenu::Build);
                    d.pinning = false;
                }
                cx.notify();
            })));
        if open && pinning {
            let rows = self.profile_versions(channel, pin.clone(), cx);
            wrap = wrap.children(menu_under("new-build", rows, cx));
        } else if open {
            let mut rows: Vec<AnyElement> = Vec::new();
            for (id, option) in [
                ("new-build-stable", Channel::Stable),
                ("new-build-nightly", Channel::Nightly),
                ("new-build-dev", Channel::Dev),
            ] {
                let unavailable = match option {
                    Channel::Stable => !self.stable_available(),
                    Channel::Dev => self.profile_build_choices(Channel::Dev).is_empty(),
                    Channel::Nightly => false,
                };
                let sub = match (option, unavailable) {
                    (Channel::Stable, true) => tr("None yet: plays Nightly until there is one"),
                    (Channel::Dev, true) => tr("No Dev build yet."),
                    _ => describe(option),
                };
                let row = menu_row(
                    id,
                    follows(option),
                    Some(sub.to_owned()),
                    pin.is_none() && channel == option,
                );
                rows.push(
                    (if option == Channel::Dev && unavailable {
                        row.opacity(0.5).cursor_default().tab_stop(false)
                    } else {
                        row.on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(d) = this.instances.dialog.as_mut().filter(|d| !d.busy) {
                                d.channel = option;
                                d.pin = None;
                                d.pinning = false;
                                d.menu = None;
                            }
                            cx.notify();
                        }))
                    })
                    .into_any_element(),
                );
            }
            rows.push(divider());
            rows.push(
                menu_row(
                    "new-build-pin",
                    tr("Pin a specific version…").into(),
                    None,
                    pin.is_some(),
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    // The same menu turns to the channel's versions.
                    if let Some(d) = this.instances.dialog.as_mut().filter(|d| !d.busy) {
                        d.pinning = true;
                    }
                    cx.notify();
                }))
                .into_any_element(),
            );
            wrap = wrap.children(menu_under("new-build", rows, cx));
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

    /// The build menu's second page: back to the channels, Follow latest, and each version of
    /// the channel to keep.
    fn profile_versions(
        &self,
        channel: Channel,
        pin: Option<String>,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let p = palette();
        let back = div()
            .id("new-versions-back")
            .reach()
            .flex()
            .items_center()
            .gap(px(6.))
            .px(px(8.))
            .py(px(6.))
            .rounded(tokens::r(tokens::R_SMALL))
            .cursor_pointer()
            .text_size(px(tokens::SMALL))
            .text_color(rgb(p.muted))
            .hover(|s| s.bg(rgb(p.hover)))
            .child(crate::glass_ui::icon(
                "arrow-left-01",
                14.,
                rgb(p.muted).into(),
            ))
            .child(channel_name(channel))
            .on_click(cx.listener(|this, _, _, cx| {
                if let Some(d) = this.instances.dialog.as_mut() {
                    d.pinning = false;
                }
                cx.notify();
            }));
        let mut rows = vec![back.into_any_element(), divider()];
        for (idx, tag) in std::iter::once(None)
            .chain(self.profile_build_choices(channel).into_iter().map(Some))
            .enumerate()
        {
            let (head, sub) = match tag.as_deref() {
                None => (
                    tr("Follow latest").to_owned(),
                    Some(describe(channel).to_owned()),
                ),
                Some(t) => {
                    let name = self.version_label(t);
                    let head = if channel == Channel::Dev {
                        trf(
                            "{build} · {commit}",
                            &[
                                ("build", &name),
                                (
                                    "commit",
                                    &sunrise_launcher::nightly::short_commit(
                                        t.rsplit_once('-').map_or(t, |(_, sha)| sha),
                                    ),
                                ),
                            ],
                        )
                    } else {
                        name
                    };
                    (head, self.build_age(t))
                }
            };
            let on = pin == tag;
            rows.push(
                menu_row(ElementId::named_usize("new-version", idx), head, sub, on)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(d) = this.instances.dialog.as_mut().filter(|d| !d.busy) {
                            d.pin = tag.clone();
                            d.pinning = false;
                            d.menu = None;
                        }
                        cx.notify();
                    }))
                    .into_any_element(),
            );
        }
        rows
    }

    /// "Start from": an empty profile, or a copy of one that exists.
    pub(super) fn profile_start_row(&mut self, cx: &mut Context<Self>) -> Div {
        let Some(d) = self.instances.dialog.as_ref() else {
            return div();
        };
        let open = d.menu == Some(DialogMenu::Start);
        let source = d.copy.as_ref().map(|c| c.source().id.clone());
        let shown = match d.copy.as_ref() {
            Some(copy) => trf("Copy “{name}”", &[("name", &copy.source().name)]),
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
                source.is_none(),
            )
            .on_click(cx.listener(|this, _, _, cx| {
                if let Some(d) = this.instances.dialog.as_mut().filter(|d| !d.busy) {
                    d.copy = None;
                    d.error = None;
                    d.menu = None;
                }
                cx.notify();
            }))
            .into_any_element()];
            let profiles: Vec<Instance> = self
                .instances
                .list
                .iter()
                .filter(|i| i.kind != instances::Kind::Pack)
                .cloned()
                .collect();
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
            wrap = wrap.children(menu_under("new-start", rows, cx));
        }
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(small_label(tr("Start from").into()))
            .child(wrap)
    }
}
