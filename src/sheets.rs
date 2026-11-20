//! The sheets: Build consent, the world question and the game-build chooser. Everything here
//! reads or acts through the existing workers:
//!
//! * **Consent** is the real `pending_consent` and `native_consent::resolve`; the registry-crate
//!   permission is the worker's own mid-build question (`Msg::NativeRegistryConsent`).
//! * **Build** edits the selected instance's pin and channel with `instances::edit`.
use super::*;
use crate::frame::button;
use crate::frame::Reach as _;
use crate::paint::palette;
use gpui::{rgb, MouseButton, MouseDownEvent};
use native_consent::PendingConsent;
use std::cell::RefCell;
use std::sync::mpsc::Sender;
use sunrise_launcher::native_mods::{Consent, UpdateCheck};

mod switch_build;
mod versions;

/// More Tab stops than any page has: how far [`Launcher::modal`] looks for the next one inside
/// the layer before it gives up.
const TAB_STOPS: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Sheet {
    /// The game-build chooser, a popover under Home's build picker.
    Build,
    /// A problem's whole text, opened from the play bar's one sentence.
    Notice,
    /// The worlds a data pack goes into (`mods::data_packs`).
    Worlds,
    /// One build to keep, picked from all of them (`versions`).
    Versions,
    SwitchBuild,
    /// Who plays, under the account chip (`account_chip`).
    AccountMenu,
    Search,
    /// The link form or the package selection it yielded.
    GithubAdd,
    /// The build window (`mods::build_modal`): closing it leaves the build running.
    ModBuild,
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
    pub(super) layered: bool,
    registry: Option<(Consent, Sender<bool>)>,
    /// The unticked boxes of the sheet on screen: native code, dependency programs.
    ticked: (bool, bool),
    error: Option<String>,
    /// What the Notice sheet says, and the way out it offers.
    notice: String,
    way_out: Option<WayOut>,
    versions: versions::State,
    switch_build: switch_build::State,
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

pub(super) fn check_key(home: &std::path::Path, to: &str) -> String {
    format!("{}{to}", native_status::fingerprint(home, Some(to)))
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

fn card() -> Div {
    div()
        .flex()
        .flex_col()
        .rounded(crate::tokens::r(crate::tokens::R_CARD))
        .border_1()
        .border_color(rgb(palette().line))
        .bg(rgb(palette().background))
}

fn muted(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(tokens::LABEL))
        .text_color(rgb(palette().muted))
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
                .rounded(crate::tokens::r(crate::tokens::R_SMALL))
                .border_1()
                .border_color(rgb(palette().muted))
                .when(on, |d| d.bg(rgb(palette().accent))),
        )
        .child(
            div()
                .text_size(px(tokens::BODY))
                .text_color(rgb(palette().text))
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
    modal_frame(id, width, body, cx, Launcher::close_sheet)
}

pub(crate) fn modal_frame(
    id: &'static str,
    width: f32,
    body: Div,
    cx: &mut Context<Launcher>,
    close: fn(&mut Launcher),
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
        .bg(palette().scrim())
        .on_mouse_down(MouseButton::Right, |_: &MouseDownEvent, _, cx| {
            cx.stop_propagation()
        })
        .on_mouse_down(MouseButton::Middle, |_: &MouseDownEvent, _, cx| {
            cx.stop_propagation()
        })
        .on_scroll(|_, _, cx| cx.stop_propagation())
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _: &MouseDownEvent, _, cx| {
                close(this);
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
                .rounded(crate::tokens::r(crate::tokens::R_PANEL))
                .border_1()
                .border_color(rgb(palette().line))
                .bg(rgb(palette().raised))
                .text_color(rgb(palette().text))
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

/// Moves the keyboard to the next Tab stop inside the layer that `focus` holds, or the one
/// before it (`back`); the page's own stops are passed over.
fn step_inside(focus: &FocusHandle, back: bool, window: &mut Window, cx: &mut App) {
    for _ in 0..TAB_STOPS {
        if back {
            window.focus_prev(cx);
        } else {
            window.focus_next(cx);
        }
        if focus.contains_focused(window, cx) {
            break;
        }
    }
    if !focus.contains_focused(window, cx) {
        window.focus(focus, cx);
    }
}

impl Launcher {
    /// The arrow keys in a layer that is a menu: the row below, or the one above (`back`).
    pub(crate) fn step_in_layer(&self, back: bool, window: &mut Window, cx: &mut App) {
        if let Some(focus) = &self.ui.sheets.focus {
            step_inside(focus, back, window, cx);
        }
    }

    /// A shortcut must not steal focus or dispatch an action behind a consent or modal.
    pub(crate) fn layer_open(&self) -> bool {
        self.ui.sheets.layered || self.ui.sheet.is_some() || self.blocking_dialog()
    }

    /// These dialogs can arrive over an already-open search. Its shortcut must not cancel
    /// another dialog's answer merely because the search is still underneath it.
    pub(crate) fn blocking_dialog(&self) -> bool {
        self.ui.sheets.registry.is_some()
            || self.pending_consent.is_some()
            || self.owner_screen_open
            || self.instances.dialog.is_some()
            || (self.ui.page == crate::frame::Page::Hosting && self.servers.create.is_some())
    }

    /// Opens `sheet`. Consent and Libraries open by themselves from their pending state.
    pub(crate) fn open_sheet(&mut self, sheet: Sheet) {
        if self.ui.sheet == Some(Sheet::GithubAdd) && sheet != Sheet::GithubAdd {
            self.github.close();
        }
        self.ui.sheets.error = None;
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
        if matches!(self.ui.sheet, Some(Sheet::SwitchBuild | Sheet::Versions)) {
            self.cancel_switch_sheet();
        }
        if let Some((_, reply)) = self.ui.sheets.registry.take() {
            let _ = reply.send(false);
        } else if self.pending_consent.is_some() {
            self.cancel_native_consent();
        }
        if self.ui.sheet.take() == Some(Sheet::GithubAdd) {
            self.github.close();
        }
        self.ui.sheets.ticked = (false, false);
        self.ui.sheets.error = None;
    }

    /// Escape: true when a sheet was open and is closed now.
    pub(crate) fn sheet_escape(&mut self) -> bool {
        let open = self.ui.sheets.registry.is_some()
            || self.pending_consent.is_some()
            || self.mods.world_ask.is_some()
            || self.ui.sheet.is_some();
        if self.mods.world_ask.is_some() {
            self.answer_world_question(sunrise_launcher::worlds::Answer::Stay);
        } else if open {
            self.close_sheet();
        }
        open
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
                    step_inside(&focus, ev.keystroke.modifiers.shift, window, cx);
                    cx.stop_propagation();
                })
                .into_any_element(),
        )
    }

    /// The sheet on screen, if any: a pending library question, a pending permission, then the
    /// sheet asked for.
    pub(crate) fn live_sheet(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<Stateful<Div>> {
        if self.ui.sheets.registry.is_some() {
            return Some(self.libraries_sheet(cx));
        }
        if self.pending_consent.is_some() {
            return Some(self.consent_sheet(cx));
        }
        if self.mods.world_ask.is_some() {
            return Some(self.world_question_sheet(cx));
        }
        if self.mods.card_error.as_ref().is_some_and(|e| e.open) {
            return Some(self.card_why_sheet(cx));
        }
        match self.ui.sheet {
            Some(Sheet::GithubAdd) => Some(self.github_add_sheet(window, cx)),
            Some(Sheet::Build) => Some(self.build_sheet(cx)),
            Some(Sheet::Notice) => Some(self.notice_sheet(cx)),
            Some(Sheet::Versions) => Some(self.versions_sheet(cx)),
            Some(Sheet::SwitchBuild) => Some(self.switch_build_sheet(cx)),
            Some(Sheet::AccountMenu) => self.account_menu(cx),
            Some(Sheet::Search) => Some(self.command_search(cx)),
            Some(Sheet::ModBuild) => {
                let sheet = self.build_modal(cx);
                if sheet.is_none() {
                    self.ui.sheet = None;
                }
                sheet
            }
            Some(Sheet::Worlds) => {
                let sheet = self.world_pick_sheet(cx);
                if sheet.is_none() {
                    self.ui.sheet = None;
                }
                sheet
            }
            None => None,
        }
    }

    // ---- Removal ----

    /// The why behind a failure, one component for the removal sheet and a card's Details: a
    /// reason per mod, the compiler's or download's log, and Copy and Save… for it.
    fn why_panel(items: Vec<(String, String)>, log: String, cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .children(items.into_iter().map(|(name, reason)| {
                div()
                    .flex()
                    .flex_col()
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(name))
                    .child(muted(reason))
            }))
            .when(!log.is_empty(), |d| {
                let (copy, save) = (log.clone(), log.clone());
                d.child(
                    div()
                        .id("alt-why-log")
                        .max_h(px(180.))
                        .overflow_y_scroll()
                        .p(px(10.))
                        .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
                        .bg(rgb(palette().field))
                        .font_family(crate::theme::MONO)
                        .text_size(px(tokens::SMALL))
                        .text_color(rgb(palette().muted))
                        .child(crate::selectable::selectable(log.clone())),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .child(button("alt-why-copy", tr("Copy")).on_click(cx.listener(
                            move |_, _: &ClickEvent, _, cx| {
                                cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                    copy.clone(),
                                ));
                            },
                        )))
                        .child(button("alt-why-save", tr("Save…")).on_click(cx.listener(
                            move |_, _: &ClickEvent, _, cx| {
                                let pick = cx.prompt_for_new_path(
                                    &backend::game_home(),
                                    Some("build-log.txt"),
                                );
                                let text = save.clone();
                                cx.spawn(async move |_, _| {
                                    if let Ok(Ok(Some(path))) = pick.await {
                                        let _ = std::fs::write(path, text);
                                    }
                                })
                                .detach();
                            },
                        ))),
                )
            })
    }

    /// A failed install's Details: the same Show why panel, for the one mod.
    fn card_why_sheet(&mut self, cx: &mut Context<Self>) -> Stateful<Div> {
        let error = self
            .mods
            .card_error
            .as_ref()
            .expect("shown only while Details is open");
        let (name, line, log, built) = (
            error.title.clone(),
            error.line.clone(),
            error.details.clone(),
            error.built,
        );
        let body = div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(title(trf(
                if built {
                    "{mod} couldn't be built"
                } else {
                    "{mod} couldn't be installed"
                },
                &[("mod", &name)],
            )))
            .child(Self::why_panel(vec![(name, line)], log, cx))
            .child(div().flex().justify_end().child(
                crate::frame::primary("alt-card-why-close", tr("Close")).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        if let Some(error) = &mut this.mods.card_error {
                            error.open = false;
                        }
                        cx.notify();
                    },
                )),
            ));
        frame("alt-card-why", 560., body, cx)
    }

    // ---- The world question ----

    /// A switch or update would remove mods that saved worlds use: the one question. Staying is
    /// the default and what closing does.
    fn world_question_sheet(&mut self, cx: &mut Context<Self>) -> Stateful<Div> {
        let ask = self
            .mods
            .world_ask
            .as_ref()
            .expect("shown only while a question is pending");
        let (question, removing) = match &ask.then {
            crate::mods_page::Then::PreparedInstall(prepared) => {
                (prepared.question().expect("prepared question"), false)
            }
            crate::mods_page::Then::PreparedEdit(prepared) => (prepared.plan.question(), true),
        };
        let body = div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(title(if removing {
                tr("This removal may affect saved worlds").into()
            } else {
                tr("This install may affect saved worlds").into()
            }))
            .children(question.lines().into_iter().map(muted))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .justify_end()
                    .gap(px(10.))
                    .child(
                        button(
                            "alt-world-switch",
                            tr(if removing {
                                "Remove anyway"
                            } else {
                                "Install anyway"
                            }),
                        )
                        .on_click(cx.listener(
                            |this, _: &ClickEvent, _, cx| {
                                this.answer_world_question(
                                    sunrise_launcher::worlds::Answer::Switch,
                                );
                                cx.notify();
                            },
                        )),
                    )
                    .child(
                        crate::frame::primary(
                            "alt-world-stay",
                            tr(if removing {
                                "Cancel removal"
                            } else {
                                "Cancel install"
                            }),
                        )
                        .on_click(cx.listener(
                            |this, _: &ClickEvent, _, cx| {
                                this.answer_world_question(sunrise_launcher::worlds::Answer::Stay);
                                cx.notify();
                            },
                        )),
                    ),
            );
        frame("alt-world-question", 560., body, cx)
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
                this.prefer_channel(channel);
                // The chooser opens under Home's build picker.
                this.navigate(crate::frame::Page::Home);
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

    pub(crate) fn request_native_build_consent(&mut self, home: std::path::PathBuf) {
        self.ui.sheets.ticked.0 = false;
        self.consent_error = None;
        let read = self.mods.begin_build_consent(home);
        self.pending_consent = Some(PendingConsent::ReadingBuild(read.clone()));
        // Snapshot the registry's own answer before hashing. An answer arriving later must
        // not replace the names for this question.
        let index = sunrise_launcher::browser::sunrise::fetched_index();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let inputs = native_status::build_inputs(&read, index.as_deref());
            let _ = tx.send(Msg::NativeConsentRead(read, inputs));
        });
    }

    /// The one warning, the first time any install would bring native code in: the plain facts
    /// and the build tools' size, with Cancel and Install. Install records the choice and goes on
    /// with the install that asked.
    fn first_native_sheet(&mut self, cx: &mut Context<Self>) -> Stateful<Div> {
        let host = sunrise_launcher::rust_toolchain::host();
        let tools = match (host, sunrise_launcher::rust_toolchain::installed()) {
            (Some(host), None) => trf(
                " The first one also downloads Sunrise's build tools (at least {size}).",
                &[("size", &storage::format_bytes(host.download_bytes()))],
            ),
            _ => String::new(),
        };
        let install = button("alt-consent-allow", tr("Install"));
        let install = match host {
            Some(_) => install.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.allow_native_consent();
                cx.notify();
            })),
            None => disabled(install),
        };
        let github = matches!(self.pending_consent, Some(PendingConsent::Github(_)));
        let body = div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(title(tr("Native mods run as part of the game").into()))
            .children(github.then(|| muted(tr(sunrise_launcher::github_mods::WARNING))))
            .child(muted(format!(
                "{}{tools}",
                tr("They can do anything a program on your computer can.")
            )))
            .children(self.consent_error.clone().map(|e| {
                div()
                    .text_color(rgb(palette().danger))
                    .text_size(px(tokens::LABEL))
                    .child(e)
            }))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .justify_end()
                    .gap(px(10.))
                    .child(
                        button("alt-consent-cancel", tr("Cancel")).on_click(cx.listener(
                            |this, _: &ClickEvent, _, cx| {
                                this.close_sheet();
                                cx.notify();
                            },
                        )),
                    )
                    .child(install),
            );
        frame("alt-consent", 540., body, cx)
    }

    fn consent_sheet(&mut self, cx: &mut Context<Self>) -> Stateful<Div> {
        if matches!(
            self.pending_consent,
            Some(PendingConsent::Install(..) | PendingConsent::Github(..))
        ) {
            return self.first_native_sheet(cx);
        }
        let pending = self.pending_consent.as_ref();
        if matches!(pending, Some(PendingConsent::ReadingBuild(_))) {
            let body = div()
                .flex()
                .flex_col()
                .child(title(tr("Checking package files…").into()))
                .child(div().flex().justify_end().child(
                    button("alt-consent-cancel", tr("Cancel")).on_click(cx.listener(
                        |this, _: &ClickEvent, _, cx| {
                            this.close_sheet();
                            cx.notify();
                        },
                    )),
                ));
            return frame("alt-consent", 760., body, cx);
        }
        let name = pending
            .map(PendingConsent::name)
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| tr("your mods").into());
        let building = matches!(pending, Some(PendingConsent::Build(_)));
        let heading = match pending {
            Some(PendingConsent::Build(inputs)) if inputs.len() == 1 => {
                tr("Build this mod?").into()
            }
            Some(PendingConsent::Build(_)) => {
                trf("Build {name} on this computer?", &[("name", &name)])
            }
            _ => trf("Allow {name} to run as native code?", &[("name", &name)]),
        };
        let rows = pending.map(PendingConsent::rows).unwrap_or_default();
        let mods = if rows.is_empty() {
            fact(tr("Mods"), name.clone(), false)
        } else {
            let packages = div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .children(rows.into_iter().map(|row| {
                    let picture = row.icon.and_then(|url| {
                        self.mods_image(&url, sunrise_launcher::browser::images::ROW)
                    });
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .child(crate::mod_icons::tile(picture, &row.id, &row.name, 32.))
                        .child(div().min_w_0().child(row.label))
                }));
            fact(tr("Mods"), packages, false)
        };
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
            .child(title(heading))
            .child(muted(tr(
                "Native mods are programs that run inside your game. Like any program you install, they can read and change your files. Allow them only if you trust who made them.",
            )))
            .child(
                card()
                    .child(mods)
                    .child(fact(tr("Downloads"), downloads, true))
                    .child(fact(
                        tr("While building"),
                        tr("If a code library the mod uses wants to run its own programs, Sunrise stops and shows you the list first. Nothing runs before you say so."),
                        true,
                    ))
                    .child(fact(
                        tr("When you play"),
                        tr("The mod, inside the game, with the same access to your files and programs as the game has."),
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
                    .map(|e| div().text_color(rgb(palette().danger)).text_size(px(tokens::LABEL)).child(e)),
            )
            .child(consent_footer(
                tr("This stays allowed on this computer until you take it back in Settings, under Mods. Progress shows beside Play on Home, and at the top of every other page."),
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
        let result = native_consent::allow(&mut self.pending_consent, true, &mut self.settings);
        match result {
            Err(error) => self.consent_error = Some(error),
            Ok(None) => {}
            Ok(Some(action)) => {
                self.consent_error = None;
                self.storage_revoke_error = None;
                self.ui.sheets.ticked = (false, false);
                match action {
                    PendingConsent::ReadingBuild(_) => {}
                    PendingConsent::Build(_) => {
                        if let Some((instance, build)) = self.native_consent_build.take() {
                            self.run_native_build(instance, build);
                        }
                    }
                    PendingConsent::Edit(request) => self.start_mod_edit(*request),
                    PendingConsent::Install(_) | PendingConsent::Github(_) => {
                        self.resume_install();
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
                .border_color(rgb(palette().divider))
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
                    .text_color(rgb(palette().danger))
                    .child(error),
                div().child(check("alt-check-again", "Check again", cx)),
            ]),
            CheckState::Done(done) => block(vec![muted(done.summary).text_size(px(tokens::SMALL))]),
        })
    }

    /// The channel is a preference. An existing committed build never changes here.
    pub(crate) fn prefer_channel(&mut self, channel: instances::Channel) {
        let Some(instance) = self.selected_instance() else {
            return;
        };
        let result = instances::edit(&instance.id, |meta| meta.channel = Some(channel));
        match result {
            Ok(_) => {
                self.ui.sheets.error = None;
                self.reload_instances();
            }
            Err(error) => self.ui.sheets.error = Some(error),
        }
    }

    fn build_sheet(&mut self, cx: &mut Context<Self>) -> Stateful<Div> {
        let Some(instance) = self.selected_instance() else {
            return div().id("alt-build");
        };
        let pinned = instance.version.clone();
        let plays = self.version_of(instance).cloned();
        let status = if pinned.is_some() {
            Some(tr("Stays on this build"))
        } else if plays.is_none() {
            let signed_in = self.accounts.selected().is_some()
                || sunrise_launcher::developer_offline::selected();
            Some(if signed_in {
                tr("Not installed yet")
            } else {
                tr("Installs after you sign in")
            })
        } else {
            None
        };
        let stable = self.stable_builds().next().is_some();
        // A different build under native mods is never activated from here: those mods have to be
        // checked against it first (the check button), and the build that follows protects worlds.
        let has_native = !native_status::enabled(&instance.home).is_empty();
        let nightly_target = self
            .target_version(None, instances::Channel::Nightly)
            .cloned();
        let stable_target = self
            .target_version(None, instances::Channel::Stable)
            .cloned();
        let gated = |target: &Option<String>| has_native && *target != plays;
        let follow_nightly = pinned.is_none() && instance.channel == instances::Channel::Nightly;
        let follow_stable = pinned.is_none() && instance.channel == instances::Channel::Stable;
        let nightly_gated = !follow_nightly && gated(&nightly_target);
        let stable_gated = stable && !follow_stable && gated(&stable_target);
        let option = |id: &'static str, on: bool, head: String, sub: String| {
            div()
                .id(id)
                .reach()
                .flex()
                .gap(px(12.))
                .p(px(10.))
                .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
                .cursor_pointer()
                .when(on, |d| d.bg(rgb(palette().selected)))
                .child(
                    div()
                        .mt(px(3.))
                        .size(px(14.))
                        .flex_none()
                        .rounded_full()
                        .border_1()
                        .border_color(rgb(if on {
                            palette().accent
                        } else {
                            palette().muted
                        }))
                        .when(on, |d| d.bg(rgb(palette().accent))),
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
                    this.prefer_channel(instances::Channel::Nightly);
                    cx.notify();
                }))
            }),
        );
        let stable_off = !stable || stable_gated;
        list = list.child(
            option(
                "alt-build-stable",
                follow_stable,
                tr("Latest Stable").into(),
                if stable {
                    tr("Only builds marked stable.").into()
                } else if follow_stable {
                    tr("No stable release yet. Plays Nightly for now.").into()
                } else {
                    tr("No stable release yet").into()
                },
            )
            .when(stable_off, |d| {
                d.opacity(0.5).cursor_default().tab_stop(false)
            })
            .when(!stable_off, |d| {
                d.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.prefer_channel(instances::Channel::Stable);
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
                    .border_color(rgb(palette().line)),
            );
        let body = div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(
                div()
                    .text_size(px(tokens::CAPTION))
                    .text_color(rgb(palette().muted))
                    .child(tr("GAME BUILD")),
            )
            .children(status.map(muted))
            .child(list)
            .children(native)
            .child(div().h(px(1.)).bg(rgb(palette().line)))
            .child(extras)
            .children(self.ui.sheets.error.clone().map(|e| {
                div()
                    .text_color(rgb(palette().danger))
                    .text_size(px(tokens::SMALL))
                    .child(e)
            }));
        // A popover under Home's build picker: the scrim is clear.
        let (left, top) = self.ui.home.chooser_at();
        let width = 372_f32.min(self.window_width - 24.);
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
                    // Never taller than the room under the picker: a long list scrolls.
                    .max_h(px((self.window_height - top - 12.).max(120.)))
                    .overflow_y_scroll()
                    .track_scroll(&crate::frame::scrolls("build-list"))
                    // Inside the window, however narrow it is.
                    .left(px(left.min(self.window_width - width - 12.).max(12.)))
                    .top(px(top))
                    .w(px(width))
                    .p(px(14.))
                    .rounded(crate::tokens::r(crate::tokens::R_CARD))
                    .border_1()
                    .border_color(rgb(palette().line))
                    .bg(rgb(palette().raised))
                    .text_color(rgb(palette().text))
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

fn fact(label: &str, value: impl IntoElement, rule: bool) -> Div {
    div()
        .flex()
        .gap(px(16.))
        .px(px(16.))
        .py(px(10.))
        .text_size(px(tokens::LABEL))
        .when(rule, |d| {
            d.border_t_1().border_color(rgb(palette().divider))
        })
        .child(
            div()
                .w(px(110.))
                .flex_none()
                .text_color(rgb(palette().muted))
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

    #[test]
    fn dated_builds_read_as_a_day() {
        assert_eq!(build_name("nightly-2026-11-17-abcdef0"), "Nov 17");
        assert_eq!(build_name("v1.0.0"), "v1.0.0");
    }
}
