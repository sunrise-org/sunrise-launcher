//! The three sheets: Resolve (a held update), Build consent and
//! the game-build chooser. Everything here reads or acts through the existing workers:
//!
//! * **Resolve** reads [`native_mods::update_check`] (or the build's worlds stop, `native_worlds`)
//!   and acts with [`Launcher::build_native_on`]; Stay writes nothing.
//! * **Consent** is the real `pending_consent` and `native_consent::resolve`; the registry-crate
//!   permission is the worker's own mid-build question (`Msg::NativeRegistryConsent`).
//! * **Build** edits the selected instance's pin and channel with `instances::edit`.
use super::*;
use crate::frame::Reach as _;
use crate::frame::{button, BAD, BG, CARD, GOLD, LINE, MUTED, SOFT, TEXT};
use gpui::{rgb, MouseButton, MouseDownEvent};
use native_consent::PendingConsent;
use std::cell::RefCell;
use std::sync::mpsc::Sender;
use sunrise_launcher::native_mods::{Consent, UpdateCheck};

mod versions;

/// More Tab stops than any page has: how far [`Launcher::modal`] looks for the next one inside
/// the layer before it gives up.
const TAB_STOPS: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Sheet {
    /// The game-build chooser, a popover over the bar's build chip.
    Build,
    /// Choose what to do with a held update.
    Resolve,
    /// A problem's whole text, opened from the play bar's one sentence.
    Notice,
    /// The worlds a data pack goes into (`mods::data_packs`).
    Worlds,
    /// One build to keep, picked from all of them (`versions`).
    Versions,
}

/// The way out of a problem the play bar raised, as the Notice sheet's one action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WayOut {
    /// The game's own files were not found: point Sunrise at the Minecraft folder.
    MinecraftFolder,
    /// The build this setup stays on can't be built for any more: follow its channel again.
    Newest(instances::Channel),
}

/// The update check for one instance and newer game, while it runs (`None`) and once it ended.
struct Memo {
    key: String,
    result: Option<Result<UpdateCheck, String>>,
}

#[derive(Default)]
pub(crate) struct SheetsState {
    /// The explicit compatibility check's state; only a click on the Build sheet starts one.
    update: RefCell<Option<Memo>>,
    /// Keyboard focus of the layer over the page, so keys never reach the page under it.
    focus: Option<FocusHandle>,
    /// What had the keyboard when that layer opened: it gets it back when the layer closes.
    opener: Option<FocusHandle>,
    /// What has the keyboard while no layer is open, read as each frame starts: the opener of
    /// the next layer, even when the page hands the keyboard to one of that layer's fields.
    before: Option<FocusHandle>,
    layered: bool,
    registry: Option<(Consent, Sender<bool>)>,
    /// The unticked boxes of the sheet on screen: native code, dependency programs.
    ticked: (bool, bool),
    error: Option<String>,
    /// The Resolve sheet shows what the check said word for word.
    details: bool,
    /// What the Notice sheet says, and the way out it offers.
    notice: String,
    way_out: Option<WayOut>,
    versions: versions::State,
}

/// The explicit native-mod check against the newer installed build.
#[derive(Clone, Debug)]
pub(crate) enum CheckState {
    /// No newer build is installed, or no native mod is turned on: nothing to check.
    None,
    /// Native mods are not allowed yet, so nothing can be checked.
    NeedsPermission(String),
    Idle(String),
    Pending(String),
    Failed(String, String),
    Done(UpdateCheck),
}

fn check_key(home: &std::path::Path, to: &str) -> String {
    format!("{}{to}", native_status::fingerprint(home, Some(to)))
}

/// A held update as the bottom bar and the Resolve sheet read it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Held {
    /// The build waiting, as people read it ("Nov 17").
    pub(crate) to: String,
    /// The build played now.
    pub(crate) plays: String,
    /// The mods that don't work with it, by id.
    pub(crate) mods: Vec<String>,
    /// What the check or the stopped build said, word for word: for a mod's author, behind
    /// "Details".
    pub(crate) details: Vec<String>,
    /// Worlds that would lose a mod's content.
    pub(crate) worlds: Vec<String>,
    /// A mod the tool refuses on any build: there is nothing to update to.
    pub(crate) blocked: bool,
    /// The raw build the update goes to (`build_native_on`).
    base: String,
    /// It is a build that already stopped for worlds, not a check.
    stopped: bool,
}

/// `Nov 17` for a dated build, else the tag.
pub(crate) fn build_name(tag: &str) -> String {
    // A game built with mods is named by the build it was made from.
    if native_mods::is_native(tag) {
        if let Some(base) = crate::glass_instance::native_base(tag) {
            return build_name(&base);
        }
    }
    sunrise_launcher::nightly::date_of(tag)
        .map(sunrise_launcher::nightly::day)
        .unwrap_or_else(|| tag.to_owned())
}

impl Held {
    fn of_check(check: UpdateCheck, plays: &str) -> Option<Self> {
        // `update_summary` is one line when every mod fits and a head plus one line per mod that
        // does not otherwise; a refusal or a world at stake is a hold as well. A fit is an offer
        // (the old Build card's), not a hold.
        let held = check.blocked || !check.worlds.is_empty() || check.summary.contains('\n');
        held.then(|| Self {
            to: build_name(&check.to),
            plays: build_name(plays),
            mods: check.off.iter().map(|(id, _)| id.clone()).collect(),
            details: check.summary.lines().map(str::to_owned).collect(),
            worlds: check.worlds,
            blocked: check.blocked,
            base: check.to,
            stopped: false,
        })
    }
}

fn card() -> Div {
    div()
        .flex()
        .flex_col()
        .rounded(px(tokens::R_CARD))
        .border_1()
        .border_color(rgb(LINE))
        .bg(rgb(BG))
}

fn muted(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(tokens::LABEL))
        .text_color(rgb(MUTED))
        .child(crate::selectable::selectable(text))
}

fn checkbox(id: &'static str, on: bool, label: String) -> Stateful<Div> {
    div()
        .id(id)
        .reach()
        .flex()
        .items_center()
        .gap(px(10.))
        .cursor_pointer()
        .child(
            div()
                .size(px(16.))
                .rounded(px(tokens::R_SMALL))
                .border_1()
                .border_color(rgb(MUTED))
                .when(on, |d| d.bg(rgb(GOLD))),
        )
        .child(
            div()
                .text_size(px(tokens::BODY))
                .text_color(rgb(TEXT))
                .child(label),
        )
}

/// The scrim and the sheet's frame. A press outside closes the sheet; inside, nothing does.
pub(crate) fn frame(
    id: &'static str,
    width: f32,
    body: Div,
    cx: &mut Context<Launcher>,
) -> Stateful<Div> {
    div()
        .id(id)
        .absolute()
        .inset_0()
        .occlude()
        .flex()
        .items_center()
        .justify_center()
        .p(px(16.))
        .bg(gpui::black().opacity(0.55))
        .on_mouse_down(MouseButton::Right, |_: &MouseDownEvent, _, cx| {
            cx.stop_propagation()
        })
        .on_mouse_down(MouseButton::Middle, |_: &MouseDownEvent, _, cx| {
            cx.stop_propagation()
        })
        .on_scroll(|_, _, cx| cx.stop_propagation())
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _: &MouseDownEvent, _, cx| {
                this.close_sheet();
                cx.notify();
            }),
        )
        .child(
            // Never taller or wider than the window: a small one scrolls the sheet.
            body.id((id, 1usize))
                .w(px(width))
                .max_w_full()
                .max_h_full()
                .overflow_y_scroll()
                .track_scroll(&crate::frame::scrolls("sheet"))
                .p(px(28.))
                .gap(px(16.))
                .rounded(px(tokens::R_PANEL))
                .border_1()
                .border_color(rgb(LINE))
                .bg(rgb(CARD))
                .text_color(rgb(TEXT))
                .on_mouse_down(MouseButton::Left, |_: &MouseDownEvent, _, cx| {
                    cx.stop_propagation()
                }),
        )
}

pub(crate) fn title(text: String) -> Div {
    div()
        .text_size(px(tokens::TITLE))
        .font_weight(FontWeight::BOLD)
        .child(crate::selectable::selectable(text))
}

impl Launcher {
    /// Opens `sheet`. Consent and Libraries open by themselves from their pending state.
    pub(crate) fn open_sheet(&mut self, sheet: Sheet) {
        self.ui.sheets.error = None;
        self.ui.sheets.details = false;
        self.ui.sheet = Some(sheet);
    }

    /// Opens a problem's whole text as a sheet, with the way out of it when there is one.
    pub(crate) fn open_notice(&mut self, text: String, way_out: Option<WayOut>) {
        self.open_sheet(Sheet::Notice);
        self.ui.sheets.notice = text;
        self.ui.sheets.way_out = way_out;
    }

    /// The worker asks before dependency build programs run. The sheet answers `reply`; losing the
    /// sheet answers no.
    pub(crate) fn registry_consent(&mut self, inventory: Consent, reply: Sender<bool>) {
        if let Some((_, old)) = self.ui.sheets.registry.replace((inventory, reply)) {
            let _ = old.send(false);
        }
        self.ui.sheets.ticked.1 = false;
    }

    /// Closes the sheet on screen the way Escape or a press outside does. Nothing is allowed,
    /// built or changed by it: a pending permission is declined, a pending library question is
    /// answered no, a held update stays held.
    pub(crate) fn close_sheet(&mut self) {
        if let Some((_, reply)) = self.ui.sheets.registry.take() {
            let _ = reply.send(false);
        } else if self.pending_consent.is_some() {
            self.cancel_native_consent();
        }
        self.ui.sheet = None;
        self.ui.sheets.ticked = (false, false);
        self.ui.sheets.error = None;
    }

    /// Escape: true when a sheet was open and is closed now.
    pub(crate) fn sheet_escape(&mut self) -> bool {
        let open = self.ui.sheets.registry.is_some()
            || self.pending_consent.is_some()
            || self.ui.sheet.is_some();
        if open {
            self.close_sheet();
        }
        open
    }

    /// The update the bar should call held, if there is one. It only *reads*: a build that stopped
    /// for worlds (`native_worlds`), or the result of the check the player started from the Build
    /// sheet ([`Self::check_native`]). It never runs a check itself, because a check stages
    /// files into the real build folder.
    pub(crate) fn held_update(&self) -> Option<Held> {
        self.held_real()
    }

    fn held_real(&self) -> Option<Held> {
        let instance = self.selected_instance()?;
        let plays = self.version_of(instance)?.clone();
        if let Some(stake) = self
            .native_worlds
            .as_ref()
            .filter(|_| self.native_for.as_deref() == Some(instance.id.as_str()))
        {
            return Some(Held {
                to: build_name(&stake.base),
                plays: build_name(&plays),
                mods: stake.mods.clone(),
                details: stake.text.split("\n\n").map(str::to_owned).collect(),
                worlds: stake.worlds.clone(),
                blocked: false,
                base: stake.base.clone(),
                stopped: true,
            });
        }
        match self.check_state() {
            CheckState::Done(check) => Held::of_check(check, &plays),
            _ => None,
        }
    }

    /// Where the selected setup's native-mod check against the newer installed build stands.
    pub(crate) fn check_state(&self) -> CheckState {
        let Some(instance) = self.selected_instance() else {
            return CheckState::None;
        };
        let Some(plays) = self.version_of(instance) else {
            return CheckState::None;
        };
        let Some(to) = native_mods::update_for(plays, &self.installed) else {
            return CheckState::None;
        };
        if native_status::enabled(&instance.home).is_empty() {
            return CheckState::None;
        }
        if self.settings.native_mods_allowed_at.is_none() {
            return CheckState::NeedsPermission(to);
        }
        let key = check_key(&instance.home, &to);
        match self
            .ui
            .sheets
            .update
            .borrow()
            .as_ref()
            .filter(|memo| memo.key == key)
        {
            None => CheckState::Idle(to),
            Some(Memo { result: None, .. }) => CheckState::Pending(to),
            Some(Memo {
                result: Some(Err(e)),
                ..
            }) => CheckState::Failed(to, e.clone()),
            Some(Memo {
                result: Some(Ok(check)),
                ..
            }) => CheckState::Done(check.clone()),
        }
    }

    /// The button's action: check the selected setup's native mods against the newer installed
    /// build, once, on a worker, in the setup's own game folder. Nothing else calls it.
    fn check_native(&mut self, cx: &mut Context<Self>) {
        if self.native_step.is_some() {
            return;
        }
        let (CheckState::Idle(to) | CheckState::Failed(to, _)) = self.check_state() else {
            return;
        };
        let Some(home) = self.selected_instance().map(|i| i.home.clone()) else {
            return;
        };
        let key = check_key(&home, &to);
        *self.ui.sheets.update.borrow_mut() = Some(Memo {
            key: key.clone(),
            result: None,
        });
        let work = cx
            .background_executor()
            .spawn(async move { native_mods::update_check(&home, &to).map_err(String::from) });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                let mut memo = this.ui.sheets.update.borrow_mut();
                // A result for a state the game folder has left is dropped.
                if memo.as_ref().is_some_and(|m| m.key == key) {
                    *memo = Some(Memo {
                        key,
                        result: Some(result),
                    });
                }
                drop(memo);
                cx.notify();
            });
        })
        .detach();
    }

    /// Reads what has the keyboard, before this frame moves it: see [`Self::modal`].
    pub(crate) fn modal_before(&mut self, window: &Window, cx: &App) {
        if !self.ui.sheets.layered {
            self.ui.sheets.before = window.focused(cx);
        }
    }

    /// The one focus rule for anything drawn over the page (a sheet, Host's Create, the
    /// sign-in): `layer` takes the keyboard when it opens, Tab stays inside it, and when it
    /// closes, by Escape or any other way, the keyboard goes back to what opened it. Escape
    /// itself is the frame's, which knows what closing each layer means.
    pub(crate) fn modal(
        &mut self,
        layer: Option<Stateful<Div>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let Some(layer) = layer else {
            self.ui.sheets.layered = false;
            if let Some(opener) = self.ui.sheets.opener.take() {
                // What opened it may have left the page since: then the frame has the keyboard.
                let still_there = self.root_focus.contains(&opener, window);
                window.focus(
                    if still_there {
                        &opener
                    } else {
                        &self.root_focus
                    },
                    cx,
                );
            }
            return None;
        };
        let focus = self
            .ui
            .sheets
            .focus
            .get_or_insert_with(|| cx.focus_handle())
            .clone();
        if !self.ui.sheets.layered {
            self.ui.sheets.layered = true;
            self.ui.sheets.opener = self.ui.sheets.before.take();
        }
        // The keyboard is somewhere on the page under the layer: the layer takes it. A field
        // the page focused for this layer, not drawn until this frame, keeps it.
        if !focus.contains_focused(window, cx) && self.root_focus.contains_focused(window, cx) {
            window.focus(&focus, cx);
        }
        Some(
            layer
                .track_focus(&focus)
                .seen()
                .on_key_down(move |ev: &KeyDownEvent, window, cx| {
                    if ev.keystroke.key != "tab" {
                        return;
                    }
                    // The next stop that is inside the layer; the page's own are passed over.
                    for _ in 0..TAB_STOPS {
                        if ev.keystroke.modifiers.shift {
                            window.focus_prev(cx);
                        } else {
                            window.focus_next(cx);
                        }
                        if focus.contains_focused(window, cx) {
                            break;
                        }
                    }
                    if !focus.contains_focused(window, cx) {
                        window.focus(&focus, cx);
                    }
                    cx.stop_propagation();
                })
                .into_any_element(),
        )
    }

    /// The sheet on screen, if any: a pending library question, a pending permission, then the
    /// sheet asked for.
    pub(crate) fn live_sheet(&mut self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        if self.ui.sheets.registry.is_some() {
            return Some(self.libraries_sheet(cx));
        }
        if self.pending_consent.is_some() {
            return Some(self.consent_sheet(cx));
        }
        match self.ui.sheet {
            Some(Sheet::Build) => Some(self.build_sheet(cx)),
            Some(Sheet::Notice) => Some(self.notice_sheet(cx)),
            Some(Sheet::Versions) => Some(self.versions_sheet(cx)),
            Some(Sheet::Worlds) => {
                let sheet = self.world_pick_sheet(cx);
                if sheet.is_none() {
                    self.ui.sheet = None;
                }
                sheet
            }
            Some(Sheet::Resolve) => match self.held_update() {
                Some(held) => Some(self.resolve_sheet(held, cx)),
                None => {
                    // Nothing is held any more (it was resolved elsewhere): close quietly.
                    self.ui.sheet = None;
                    None
                }
            },
            None => None,
        }
    }

    // ---- Resolve ----

    /// An update is out and some mods don't work with it yet: which, and the two ways on.
    /// Staying is the default and what closing does; nothing here runs until a button is pressed.
    fn resolve_sheet(&mut self, held: Held, cx: &mut Context<Self>) -> Stateful<Div> {
        let Held {
            to,
            plays,
            mods,
            details,
            worlds,
            blocked,
            base,
            ..
        } = held;
        let known = self.mods_named(&mods);
        let names: Vec<String> = known.iter().map(|(name, _)| name.clone()).collect();
        let them = match names.as_slice() {
            [one] => one.clone(),
            _ => tr("them").to_string(),
        };
        let heading = match (blocked, names.as_slice()) {
            (true, _) => trf("{to} is out, but one of your mods stops it", &[("to", &to)]),
            (false, [one]) => trf(
                "{to} is out, but {mod} doesn't work with it yet",
                &[("to", &to), ("mod", one)],
            ),
            (false, many) => trf(
                "{to} is out, but {count} of your mods don't work with it yet",
                &[("to", &to), ("count", &many.len().to_string())],
            ),
        };
        let lead = if blocked {
            trf(
                "One of your mods can't be built for any game build. Remove it under Mods, or wait for its author. Until then you keep playing {plays}.",
                &[("plays", &plays)],
            )
        } else {
            trf(
                "You can keep playing {plays} and wait, or update without {them}.",
                &[("plays", &plays), ("them", &them)],
            )
        };

        // The mods, each with what can be done about it right here.
        let mut can_update = false;
        let mut list = div()
            .flex()
            .flex_col()
            .rounded(px(tokens::R_CARD))
            .border_1()
            .border_color(rgb(LINE))
            .bg(rgb(BG));
        for (n, (id, (name, newer))) in mods.iter().zip(&known).enumerate() {
            let about = match newer {
                Some(version) => trf(
                    "Version {version} is out. Update it, then check the game update again.",
                    &[("version", version)],
                ),
                None => tr(
                    "Needs an update from its author. There is no newer version yet; when there is, it shows here and under Mods.",
                )
                .to_string(),
            };
            let update = newer.as_ref().map(|_| {
                can_update = true;
                let id = id.clone();
                button(
                    ElementId::named_usize("alt-resolve-mod", n),
                    trf("Update {mod}", &[("mod", name)]),
                )
                .bg(rgb(TEXT))
                .border_color(rgb(TEXT))
                .text_color(rgb(BG))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.close_sheet();
                    this.update_mod(&id);
                    cx.notify();
                }))
            });
            list = list.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .p(px(14.))
                    .when(n > 0, |d| d.border_t_1().border_color(rgb(LINE)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(div().font_weight(FontWeight::SEMIBOLD).child(name.clone()))
                            .child(muted(about)),
                    )
                    .children(update),
            );
        }

        // What updating without them does, said before the button that does it.
        let without = (!blocked).then(|| {
            let kept = trf(
                "Updating without {them} turns it off; it isn't removed, and you can turn it back on when it fits.",
                &[("them", &them)],
            );
            match worlds.as_slice() {
                [] => kept,
                saved => format!(
                    "{kept} {}",
                    trf(
                        "{worlds} is backed up first: opened without the mod, a world loses what the mod added to it, for good.",
                        &[("worlds", &saved.join(", "))],
                    )
                ),
            }
        });

        let stay = button(
            "alt-resolve-stay",
            trf("Stay on {build} for now", &[("build", &plays)]),
        )
        // Recommended, unless a mod can simply be updated.
        .when(!can_update, |b| {
            b.bg(rgb(TEXT)).border_color(rgb(TEXT)).text_color(rgb(BG))
        })
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
            // Staying is what already happens: nothing is written.
            this.close_sheet();
            cx.notify();
        }));
        let go = (!blocked).then(|| {
            let back_up = !worlds.is_empty();
            button(
                "alt-resolve-update",
                trf("Update without {them}", &[("them", &them)]),
            )
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.close_sheet();
                // With worlds at stake the worker backs each up and checks the backup before
                // anything is turned off; a backup that fails stops with nothing changed.
                this.build_native_on(Some(base.clone()), back_up);
                cx.notify();
            }))
        });

        let open = self.ui.sheets.details;
        let more = div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                div()
                    .id("alt-resolve-details")
                    .tab_index(0)
                    .seen()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_size(px(tokens::SMALL))
                    .text_color(rgb(MUTED))
                    .cursor_pointer()
                    .child(blocks::icon(
                        if open {
                            "chevron-down"
                        } else {
                            "chevron-right"
                        },
                        11.,
                        rgb(MUTED).into(),
                    ))
                    .child(tr("Details"))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.ui.sheets.details = !this.ui.sheets.details;
                        cx.notify();
                    })),
            )
            .children(open.then(|| {
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .p(px(12.))
                    .rounded(px(tokens::R_CONTROL))
                    .bg(rgb(BG))
                    .font_family(crate::theme::MONO)
                    .text_size(px(tokens::CAPTION))
                    .text_color(rgb(MUTED))
                    .children(details)
            }));

        let body = div()
            .flex()
            .flex_col()
            .child(title(heading))
            .child(div().text_size(px(tokens::BODY)).child(lead))
            .children((!mods.is_empty()).then_some(list))
            .children(without.map(muted))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .justify_end()
                    .gap(px(8.))
                    .children(go)
                    .child(stay),
            )
            .child(more);
        frame("alt-resolve", 580., body, cx)
    }

    /// What went wrong, in full: each paragraph on its own, with one way out.
    fn notice_sheet(&mut self, cx: &mut Context<Self>) -> Stateful<Div> {
        let paragraphs: Vec<Div> = self
            .ui
            .sheets
            .notice
            .split('\n')
            .filter(|p| !p.trim().is_empty())
            .map(|p| div().text_size(px(tokens::LABEL)).child(p.to_owned()))
            .collect();
        let main = |label: SharedString| crate::frame::primary("alt-notice-way-out", label);
        let way_out = self.ui.sheets.way_out.map(|way| match way {
            WayOut::MinecraftFolder => main(tr("Choose Minecraft folder").into()).on_click(
                cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.close_sheet();
                    this.choose_folder(cx);
                    cx.notify();
                }),
            ),
            // One press follows the channel again, and the build sheet opens on what that chose.
            WayOut::Newest(channel) => main(match channel {
                instances::Channel::Stable => tr("Use the newest Stable").into(),
                instances::Channel::Nightly => tr("Use the newest Nightly").into(),
            })
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.native_error = None;
                this.pin_build(None, channel);
                this.open_sheet(Sheet::Build);
                cx.notify();
            })),
        });
        let body = div()
            .flex()
            .flex_col()
            .child(title(tr("What went wrong").into()))
            .child(div().flex().flex_col().gap(px(10.)).children(paragraphs))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .justify_end()
                    .gap(px(8.))
                    .child(
                        button("alt-notice-close", tr("Close")).on_click(cx.listener(
                            |this, _: &ClickEvent, _, cx| {
                                this.close_sheet();
                                cx.notify();
                            },
                        )),
                    )
                    .children(way_out),
            );
        frame("alt-notice", 560., body, cx)
    }

    // ---- Consent ----

    fn consent_sheet(&mut self, cx: &mut Context<Self>) -> Stateful<Div> {
        let pending = self.pending_consent.as_ref();
        let name = pending
            .map(PendingConsent::name)
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| tr("your mods").into());
        let building = matches!(
            pending,
            Some(PendingConsent::Build(_) | PendingConsent::Local(_))
        );
        let host = sunrise_launcher::rust_toolchain::host();
        let downloads = match (host, sunrise_launcher::rust_toolchain::installed()) {
            (None, _) => tr("Unavailable on this platform").into(),
            (Some(_), Some(_)) => tr("Nothing: the build tools are already here").into(),
            (Some(host), None) => trf(
                "{size} of build tools from Sunrise",
                &[("size", &storage::format_bytes(host.download_bytes()))],
            ),
        };
        let ticked = self.ui.sheets.ticked.0;
        let can = host.is_some() && ticked;
        let allow = button(
            "alt-consent-allow",
            if building {
                tr("Allow and build")
            } else {
                tr("Allow")
            },
        );
        let allow = if can {
            allow.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.allow_native_consent();
                cx.notify();
            }))
        } else {
            disabled(allow)
        };
        let body = div()
            .flex()
            .flex_col()
            .child(title(if building {
                trf("Build {name} on this computer?", &[("name", &name)])
            } else {
                trf("Allow {name} to run as native code?", &[("name", &name)])
            }))
            .child(muted(tr(
                "Native mods are programs that run inside your game. Like any program you install, they can read and change your files. Allow them only if you trust who made them.",
            )))
            .child(
                card()
                    .child(fact(tr("Mods"), name.clone(), false))
                    .child(fact(tr("Downloads"), downloads, true))
                    .child(fact(
                        tr("While building"),
                        tr("If a code library the mod uses wants to run its own programs, Sunrise stops and shows you the list first. Nothing runs before you say so.").into(),
                        true,
                    ))
                    .child(fact(
                        tr("When you play"),
                        tr("The mod, inside the game, with the same access to your files and programs as the game has.").into(),
                        true,
                    )),
            )
            .child(
                checkbox(
                    "alt-consent-native",
                    ticked,
                    trf("Let {name} run as native code in my game", &[("name", &name)]),
                )
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.ui.sheets.ticked.0 ^= true;
                    cx.notify();
                })),
            )
            .children(
                self.consent_error
                    .clone()
                    .map(|e| div().text_color(rgb(BAD)).text_size(px(tokens::LABEL)).child(e)),
            )
            .child(consent_footer(
                tr("This stays allowed on this computer until you take it back in Settings, under Mods. Progress shows in the bar below."),
                button("alt-consent-cancel", tr("Not now")).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        this.close_sheet();
                        cx.notify();
                    },
                )),
                allow,
            ));
        frame("alt-consent", 760., body, cx)
    }

    /// Allow: persist the permission, then run the one pending action. The old overlay's match.
    fn allow_native_consent(&mut self) {
        let result = native_consent::resolve(
            &mut self.pending_consent,
            true,
            true,
            &mut self.settings,
            &self.mods.home(),
        );
        match result {
            Err(error) => self.consent_error = Some(error),
            Ok(None) => {}
            Ok(Some(action)) => {
                self.consent_error = None;
                self.storage_revoke_error = None;
                self.ui.sheets.ticked = (false, false);
                match action {
                    PendingConsent::Build(_) => {
                        let (base, back_up) = self.native_consent_build.take().unwrap_or_default();
                        self.build_native_on(base, back_up)
                    }
                    PendingConsent::Toggle(entry) => self.mods_switch(&entry),
                    PendingConsent::Install(source, id, known) => {
                        self.mods_install(source, id, known)
                    }
                    PendingConsent::Github(found, only) => self.github_install(found, only),
                    PendingConsent::Local(id) => {
                        self.mods.message = sunrise_launcher::browser::java::set_enabled(
                            &self.mods.home(),
                            &id,
                            true,
                        )
                        .err()
                        .map(Err);
                        self.instance_mods_changed();
                    }
                }
            }
        }
    }

    // ---- Libraries ----

    fn libraries_sheet(&mut self, cx: &mut Context<Self>) -> Stateful<Div> {
        let (inventory, _) = self
            .ui
            .sheets
            .registry
            .as_ref()
            .expect("shown only while a question is pending");
        let mut names: Vec<&String> = inventory
            .build_scripts
            .iter()
            .chain(&inventory.proc_macros)
            .collect();
        names.sort();
        names.dedup();
        let count = names.len();
        let heading = inventory.label();
        let rows = card()
            .child(fact(
                tr("Build scripts"),
                list_or_none(&inventory.build_scripts),
                false,
            ))
            .child(fact(
                tr("Proc macros"),
                list_or_none(&inventory.proc_macros),
                true,
            ));
        let ticked = self.ui.sheets.ticked.1;
        let go = button("alt-libraries-allow", tr("Allow and continue"));
        let go = if ticked {
            go.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                if let Some((_, reply)) = this.ui.sheets.registry.take() {
                    let _ = reply.send(true);
                }
                this.ui.sheets.ticked.1 = false;
                cx.notify();
            }))
        } else {
            disabled(go)
        };
        let body = div()
            .flex()
            .flex_col()
            .child(title(heading))
            .child(muted(tr(
                "Registry dependencies are native code. Their build scripts and proc macros run during compilation, before the game starts, with the same access to your files as the launcher has.",
            )))
            .child(rows)
            .child(
                checkbox(
                    "alt-libraries-tick",
                    ticked,
                    trf(
                        "Let those {count} libraries run their programs while building",
                        &[("count", &count.to_string())],
                    ),
                )
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.ui.sheets.ticked.1 ^= true;
                    cx.notify();
                })),
            )
            .child(consent_footer(
                tr("Not now stops the build. Nothing has run."),
                button("alt-libraries-cancel", tr("Not now")).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        this.close_sheet();
                        cx.notify();
                    },
                )),
                go,
            ));
        frame("alt-libraries", 760., body, cx)
    }

    // ---- Game build ----

    /// The Build sheet's native-mod compatibility section: the explicit check and where it stands.
    fn native_check_block(&mut self, cx: &mut Context<Self>) -> Option<Div> {
        let state = self.check_state();
        let block = |lines: Vec<Div>| {
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .pt(px(10.))
                .border_t_1()
                .border_color(rgb(SOFT))
                .children(lines)
        };
        let check = |id: &'static str, label: &str, cx: &mut Context<Self>| {
            button(id, tr(label)).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.check_native(cx);
                cx.notify();
            }))
        };
        Some(match state {
            CheckState::None => return None,
            CheckState::NeedsPermission(to) => block(vec![muted(trf(
                "A newer build, {to}, is installed. Native mods need your permission before they can be checked against it.",
                &[("to", &build_name(&to))],
            ))
            .text_size(px(tokens::SMALL))]),
            CheckState::Idle(to) => block(vec![
                muted(trf(
                    "A newer build, {to}, is installed. Your native mods have not been checked against it.",
                    &[("to", &build_name(&to))],
                ))
                .text_size(px(tokens::SMALL)),
                div().child(check(
                    "alt-check",
                    "Check native mod compatibility",
                    cx,
                )),
            ]),
            CheckState::Pending(to) => block(vec![muted(trf(
                "Checking your native mods against {to}…",
                &[("to", &build_name(&to))],
            ))
            .text_size(px(tokens::SMALL))]),
            CheckState::Failed(_, error) => block(vec![
                div()
                    .text_size(px(tokens::SMALL))
                    .text_color(rgb(BAD))
                    .child(error),
                div().child(check("alt-check-again", "Check again", cx)),
            ]),
            CheckState::Done(done) => match Held::of_check(done.clone(), "") {
                Some(_) => block(vec![
                    muted(trf(
                        "The update to {to} is on hold: some mods don't fit.",
                        &[("to", &build_name(&done.to))],
                    ))
                    .text_size(px(tokens::SMALL)),
                    div().child(
                        button("alt-check-resolve", tr("Resolve…")).on_click(cx.listener(
                            |this, _: &ClickEvent, _, cx| {
                                this.open_sheet(Sheet::Resolve);
                                cx.notify();
                            },
                        )),
                    ),
                ]),
                None => {
                    let base = done.to.clone();
                    block(vec![
                        muted(done.summary.clone()).text_size(px(tokens::SMALL)),
                        div().child(
                            button(
                                "alt-check-build",
                                trf("Build on {to}", &[("to", &build_name(&done.to))]),
                            )
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.close_sheet();
                                this.build_native_on(Some(base.clone()), false);
                                cx.notify();
                            })),
                        ),
                    ])
                }
            },
        })
    }

    pub(crate) fn pin_build(&mut self, version: Option<String>, channel: instances::Channel) {
        let Some(instance) = self.selected_instance() else {
            return;
        };
        let id = instance.id.clone();
        // Backstop for the sheet's own gate: never pin another build under native mods.
        if let (Some(to), Some(plays)) = (&version, self.version_of(instance)) {
            if to != plays && !native_status::enabled(&instance.home).is_empty() {
                self.ui.sheets.error =
                    Some(tr("Check your native mods against that build first.").into());
                return;
            }
        }
        match instances::edit(&id, |m| {
            m.version = version;
            m.channel = Some(channel);
        }) {
            Ok(_) => {
                self.ui.sheets.error = None;
                self.reload_instances();
            }
            Err(e) => self.ui.sheets.error = Some(e),
        }
    }

    fn build_sheet(&mut self, cx: &mut Context<Self>) -> Stateful<Div> {
        let Some(instance) = self.selected_instance() else {
            return div().id("alt-build");
        };
        let pinned = instance.version.clone();
        let plays = self.version_of(instance).cloned();
        let stable = self.stable_builds().iter().any(|b| b.week.is_none());
        // A different build under native mods is never activated from here: those mods have to be
        // checked against it first (the check button), and the build that follows protects worlds.
        let has_native = !native_status::enabled(&instance.home).is_empty();
        let nightly_target = self.launcher_version().cloned();
        let stable_target = self
            .stable_builds()
            .iter()
            .find_map(|b| self.installed.iter().find(|t| **t == b.tag))
            .cloned()
            .or_else(|| nightly_target.clone());
        let gated = |target: &Option<String>| has_native && *target != plays;
        let follow_nightly = pinned.is_none() && instance.channel == instances::Channel::Nightly;
        let follow_stable = pinned.is_none() && instance.channel == instances::Channel::Stable;
        let nightly_gated = !follow_nightly && gated(&nightly_target);
        let stable_gated = !follow_stable && gated(&stable_target);
        let option = |id: &'static str, on: bool, head: String, sub: String| {
            div()
                .id(id)
                .reach()
                .flex()
                .gap(px(12.))
                .p(px(10.))
                .rounded(px(tokens::R_CONTROL))
                .cursor_pointer()
                .when(on, |d| d.bg(rgb(BG)))
                .child(
                    div()
                        .mt(px(3.))
                        .size(px(14.))
                        .flex_none()
                        .rounded_full()
                        .border_1()
                        .border_color(rgb(if on { GOLD } else { MUTED }))
                        .when(on, |d| d.bg(rgb(GOLD))),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(div().text_size(px(tokens::BODY)).child(head))
                        .child(muted(sub).text_size(px(tokens::SMALL))),
                )
        };
        // Three choices, however many builds there are: the two that follow, and one build
        // picked in its own sheet.
        let mut list = div().flex().flex_col().gap(px(2.));
        list = list.child(
            option(
                "alt-build-nightly",
                follow_nightly,
                tr("Latest Nightly").into(),
                tr("The newest build each day.").into(),
            )
            .when(nightly_gated, |d| {
                d.opacity(0.5).cursor_default().tab_stop(false)
            })
            .when(!nightly_gated, |d| {
                d.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.pin_build(None, instances::Channel::Nightly);
                    cx.notify();
                }))
            }),
        );
        let stable_off = (!stable && !follow_stable) || stable_gated;
        list = list.child(
            option(
                "alt-build-stable",
                follow_stable,
                tr("Latest Stable").into(),
                if stable || follow_stable {
                    tr("Only builds marked stable.").into()
                } else {
                    tr("No stable release yet").into()
                },
            )
            .when(stable_off, |d| {
                d.opacity(0.5).cursor_default().tab_stop(false)
            })
            .when(!stable_off, |d| {
                d.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.pin_build(None, instances::Channel::Stable);
                    cx.notify();
                }))
            }),
        );
        list = list.child(
            option(
                "alt-build-keep",
                pinned.is_some(),
                tr("A specific version…").into(),
                match &pinned {
                    Some(tag) => sunrise_launcher::nightly::label(tag)
                        .map(|l| format!("{} · {l}", build_name(tag)))
                        .unwrap_or_else(|| build_name(tag)),
                    None => tr("Pick one build and keep it until you change this.").into(),
                },
            )
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.open_versions();
                cx.notify();
            })),
        );
        // Said once, and only when native mods stop a choice above; the check sits with it.
        let native = self.native_check_block(cx).or_else(|| {
            (nightly_gated || stable_gated).then(|| {
                muted(tr(
                    "Your native mods are made for the build you play. Another build needs them checked first.",
                ))
                .text_size(px(tokens::SMALL))
                .px(px(10.))
            })
        });
        let extras = div()
            .flex()
            .items_start()
            .justify_between()
            .gap(px(12.))
            .opacity(0.5)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(div().text_size(px(tokens::BODY)).child(tr("Sunrise Extras")))
                    .child(
                        muted(tr(
                            "Sunrise’s additions on top of vanilla. This switch arrives with the build that supports it.",
                        ))
                        .text_size(px(tokens::SMALL)),
                    ),
            )
            .child(
                div()
                    .mt(px(2.))
                    .flex_none()
                    .w(px(34.))
                    .h(px(20.))
                    .rounded_full()
                    .border_1()
                    .border_color(rgb(LINE)),
            );
        let body = div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(
                div()
                    .text_size(px(tokens::CAPTION))
                    .text_color(rgb(MUTED))
                    .child(tr("GAME BUILD")),
            )
            .child(list)
            .children(native)
            .child(div().h(px(1.)).bg(rgb(LINE)))
            .child(extras)
            .children(self.ui.sheets.error.clone().map(|e| {
                div()
                    .text_color(rgb(BAD))
                    .text_size(px(tokens::SMALL))
                    .child(e)
            }));
        // A popover over the bar's build chip: the scrim is clear.
        div()
            .id("alt-build")
            .absolute()
            .inset_0()
            .occlude()
            .on_mouse_down(MouseButton::Right, |_: &MouseDownEvent, _, cx| {
                cx.stop_propagation()
            })
            .on_mouse_down(MouseButton::Middle, |_: &MouseDownEvent, _, cx| {
                cx.stop_propagation()
            })
            .on_scroll(|_, _, cx| cx.stop_propagation())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, _, cx| {
                    this.close_sheet();
                    cx.notify();
                }),
            )
            .child(
                body.id("alt-build-body")
                    .absolute()
                    // Never taller than the room above the bar: a long list scrolls.
                    .max_h(px((self.window_height - 94.).max(120.)))
                    .overflow_y_scroll()
                    .track_scroll(&crate::frame::scrolls("build-list"))
                    // Under the build chip, which starts the bar when nobody is signed in.
                    .left(px(if self.accounts.selected().is_some() {
                        148.
                    } else {
                        16.
                    }))
                    .bottom(px(78.))
                    .w(px(372.))
                    .p(px(14.))
                    .rounded(px(tokens::R_CARD))
                    .border_1()
                    .border_color(rgb(LINE))
                    .bg(rgb(CARD))
                    .text_color(rgb(TEXT))
                    .on_mouse_down(MouseButton::Left, |_: &MouseDownEvent, _, cx| {
                        cx.stop_propagation()
                    }),
            )
    }
}

fn list_or_none(names: &[String]) -> String {
    if names.is_empty() {
        tr("None").into()
    } else {
        names.join(", ")
    }
}

fn fact(label: &str, value: String, rule: bool) -> Div {
    div()
        .flex()
        .gap(px(16.))
        .px(px(16.))
        .py(px(10.))
        .text_size(px(tokens::LABEL))
        .when(rule, |d| d.border_t_1().border_color(rgb(SOFT)))
        .child(
            div()
                .w(px(110.))
                .flex_none()
                .text_color(rgb(MUTED))
                .child(label.to_owned()),
        )
        .child(div().flex_1().min_w_0().child(value))
}

fn consent_footer(note: &str, cancel: Stateful<Div>, go: Stateful<Div>) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(12.))
        .child(muted(note.to_owned()))
        .child(
            div()
                .flex()
                .flex_wrap()
                .justify_end()
                .gap(px(10.))
                .child(cancel)
                .child(go),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(summary: String, worlds: &[&str], blocked: bool) -> UpdateCheck {
        UpdateCheck {
            to: "nightly-2026-11-17-abcdef0".into(),
            summary,
            worlds: worlds.iter().map(|w| w.to_string()).collect(),
            blocked,
            off: Vec::new(),
        }
    }

    #[test]
    fn a_fit_is_an_offer_and_a_misfit_is_a_hold() {
        let fits = native_mods::fit::update_summary("Nov 17", 2, &[]);
        assert!(Held::of_check(check(fits, &[], false), "nightly-2026-11-14-0000000").is_none());
        let off = [("jei".to_string(), "made for another game".to_string())];
        let summary = native_mods::fit::update_summary("Nov 17", 2, &off);
        let mut misfit = check(summary, &[], false);
        misfit.off = off.to_vec();
        let held = Held::of_check(misfit, "nightly-2026-11-14-0000000")
            .expect("a mod that does not fit holds the update");
        assert_eq!(
            (held.to.as_str(), held.plays.as_str()),
            ("Nov 17", "Nov 14")
        );
        assert_eq!(held.mods, ["jei"]);
        assert!(held.details[1].starts_with("jei does not"));
    }

    #[test]
    fn a_refusal_or_a_world_at_stake_holds_even_on_one_line() {
        let refused = Held::of_check(check("one mod is refused".into(), &[], true), "x").unwrap();
        assert!(refused.blocked);
        let worlds = Held::of_check(check("Move?".into(), &["Harbor"], false), "x").unwrap();
        assert_eq!(worlds.worlds, ["Harbor"]);
    }

    #[test]
    fn dated_builds_read_as_a_day() {
        assert_eq!(build_name("nightly-2026-11-17-abcdef0"), "Nov 17");
        assert_eq!(build_name("v1.0.0"), "v1.0.0");
    }
}
