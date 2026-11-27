//! The sheets: Resolve (a held update) and the game-build chooser. Everything here reads or
//! acts through the existing workers:
//!
//! * **Resolve** handles the same-base native build’s world stop; Stay writes nothing.
//! * **Build** picks a channel or an exact build. Both use the staged switch.
use super::*;
use crate::frame::Reach as _;
use crate::paint::palette;
use crate::widgets::button::{primary, secondary};
use gpui::{rgb, MouseButton, MouseDownEvent};
use std::sync::mpsc::Sender;

mod build_failure;
mod build_menu;
mod build_names;
mod chrome;
mod notice;
mod provider_choice;
mod publisher_change;
mod resolve;
mod switching;
pub(crate) use switching::PortSwitch;
mod versions;
#[cfg(test)]
use build_menu::build_menu_error;
pub(crate) use build_names::*;
pub(crate) use chrome::*;
pub(crate) use versions::bench_wheel;

/// More Tab stops than any page has: how far [`Launcher::modal`] looks for the next one inside
/// the layer before it gives up.
const TAB_STOPS: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Sheet {
    /// The game-build chooser, a popover under Home's build picker.
    Build,
    /// Choose what to do with a held update.
    Resolve,
    /// A problem's whole text, opened from the play bar's one sentence.
    Notice,
    /// The worlds a data pack goes into (`mods::data_packs`).
    Worlds,
    /// One build to keep, picked from all of them (`versions`).
    Versions,
    Switching,
    SwitchProvider,
    BuildChanges,
    /// Who plays, under the account chip (`account_chip`).
    AccountMenu,
    LauncherUpdate,
    /// The selected profile's options (Sunrise Extras, its folder), a popover under Home's
    /// menu button.
    ProfileMenu,
    Search,
    /// The build window (`mods::build_modal`): closing it leaves the build running.
    ModBuild,
    /// Play was accepted while the registry worker is proving the new generation.
    RegistryPlay,
    GithubAdd,
    NightlySourceLog,
    /// Why an Add or Update was refused (`mods::refused`).
    Refused,
    /// The builds a registry release works on (`mods::release_builds`).
    ReleaseBuilds,
}

/// The way out of a problem the play bar raised, as the Notice sheet's one action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum WayOut {
    /// The game's own files were not found: point Sunrise at the Minecraft folder.
    MinecraftFolder,
    ProfileFolder(std::path::PathBuf),
    /// The build this setup stays on can't be built for any more: follow its channel again.
    Newest(instances::Channel),
}

#[derive(Default)]
pub(crate) struct SheetsState {
    /// Keyboard focus of the layer over the page, so keys never reach the page under it.
    focus: Option<FocusHandle>,
    /// What had the keyboard when that layer opened: it gets it back when the layer closes.
    opener: Option<FocusHandle>,
    /// What has the keyboard while no layer is open, read as each frame starts: the opener of
    /// the next layer, even when the page hands the keyboard to one of that layer's fields.
    before: Option<FocusHandle>,
    pub(super) layered: bool,
    error: Option<String>,
    /// The Resolve sheet shows what the check said word for word.
    details: bool,
    /// What the Notice sheet says, and the way out it offers.
    notice: String,
    way_out: Option<WayOut>,
    versions: versions::State,
    /// The build chooser's open submenu.
    build_sub: build_menu::Sub,
    pub(super) switching: switching::State,
    publisher_change: Option<publisher_change::PendingPublisherChange>,
    provider_choice: Option<provider_choice::PendingProviderChoice>,
    pub(super) profile_action: Option<crate::profile_delete::Action>,
    /// A build waits on the unreviewed-code warning for native mods nobody reviewed.
    pub(crate) unreviewed: Option<crate::github_mods_page::PendingUnreviewed>,
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

#[cfg(test)]
#[test]
fn dated_game_builds_share_the_button_and_picker_labels() {
    use instances::Channel;
    let dev = "dev-2026-11-22-7cfdf81";
    let nightly = "nightly-2026-11-20-aaaaaaa";
    assert_eq!(build_name(dev), "Dev · Nov 22");
    assert_eq!(build_name(nightly), "Nightly · Nov 20");
    assert_eq!(build_name("26.3-0.1.0"), "Stable · 0.1.0");
    assert_eq!(build_name_with_commit(dev), "Dev · Nov 22 (7cfdf81)");
    assert_eq!(build_name_with_commit("26.3-0.1.0"), "Stable · 0.1.0");
    assert_eq!(channel_build(dev), Some((Channel::Dev, "Nov 22".into())));
    assert_eq!(
        channel_build(nightly),
        Some((Channel::Nightly, "Nov 20".into()))
    );
    assert_eq!(
        channel_build("26.3-0.1.0"),
        Some((Channel::Stable, "0.1.0".into()))
    );
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
        self.ui.sheets.publisher_change.is_some()
            || self.ui.sheets.provider_choice.is_some()
            || self.ui.sheets.unreviewed.is_some()
            || self.source_question().is_some()
            || self.mods.installs.question.is_some()
            || self.mods.installs.world.is_some()
            || self.sign_in_screen.open
            || self.instances.dialog.is_some()
            || self.ui.sheets.profile_action.is_some()
            || self.worlds.action.is_some()
            || (self.ui.page == crate::frame::Page::Hosting && self.servers.create.is_some())
    }

    /// Opens `sheet`. A pending question opens its own sheet.
    pub(crate) fn open_sheet(&mut self, sheet: Sheet) {
        self.close_menus();
        self.ui.sheets.error = None;
        self.ui.sheets.details = false;
        self.ui.sheet = Some(sheet);
    }

    /// Closes every open menu and popover: the sheets that hang from a control (the build
    /// chooser, the profile and account menus, the update card), the account chip's submenu, the
    /// profile's action menu, the Settings account and language menus, the Mods filter dropdowns,
    /// a server row's menu and the profile dialog's dropdown (the dialog itself stays). Every
    /// action that opens a menu calls this first, so a new menu replaces the old one and two are
    /// never up together.
    pub(crate) fn close_menus(&mut self) {
        crate::glass_ui::menus_changed();
        if matches!(
            self.ui.sheet,
            Some(Sheet::Build | Sheet::ProfileMenu | Sheet::AccountMenu | Sheet::LauncherUpdate)
        ) {
            self.ui.sheet = None;
        }
        self.account_menu.sub = None;
        self.ui.sheets.build_sub = build_menu::Sub::default();
        if matches!(
            self.ui.sheets.profile_action,
            Some(crate::profile_delete::Action::Menu { .. })
        ) {
            self.close_profile_action();
        }
        self.ui.settings.close_menus();
        self.worlds.close_menu();
        self.mods_state.close_menu();
        self.multiplayer.menu = None;
        if let Some(dialog) = self.instances.dialog.as_mut() {
            dialog.menu = None;
        }
    }

    /// Opens a problem's whole text as a sheet, with the way out of it when there is one.
    pub(crate) fn open_notice(&mut self, text: String, way_out: Option<WayOut>) {
        self.open_sheet(Sheet::Notice);
        self.ui.sheets.notice = text;
        self.ui.sheets.way_out = way_out;
    }

    pub(crate) fn reset_install_question_sheet(&mut self) {
        self.ui.sheets.details = false;
        self.ui.sheets.error = None;
    }

    /// Closes the sheet on screen the way Escape or a press outside does. Nothing is allowed,
    /// built or changed by it: a pending question is answered no, a held update stays held.
    pub(crate) fn close_sheet(&mut self) {
        if self.ui.sheet == Some(Sheet::GithubAdd) {
            self.github.close();
        }
        if matches!(
            self.ui.sheet,
            Some(Sheet::Switching | Sheet::SwitchProvider)
        ) {
            self.hide_switch();
        }
        if let Some(pending) = self.ui.sheets.unreviewed.take() {
            pending.refuse();
        } else if self.dismiss_publisher_change() || self.answer_provider_choice(None) {
            // Keep the current version and remembered publisher; install no provider.
        } else if self.source_question().is_some() {
            self.answer_source_question(Ok(false));
        } else if let Some(reply) = self
            .mods
            .installs
            .question
            .take()
            .map(|(_, reply)| reply)
            .or_else(|| self.mods.installs.world.take().map(|(_, reply)| reply))
        {
            let _ = reply.send(false);
        }
        self.ui.sheet = None;
        self.ui.sheets.error = None;
    }

    /// Escape: true when a sheet was open and is closed now.
    pub(crate) fn sheet_escape(&mut self) -> bool {
        let prompt_open = self.ui.sheets.publisher_change.is_some()
            || self.ui.sheets.provider_choice.is_some()
            || self.ui.sheets.unreviewed.is_some()
            || self.source_question().is_some()
            || self.mods.installs.question.is_some()
            || self.mods.installs.world.is_some();
        if !prompt_open && self.ui.sheets.profile_action.is_some() {
            self.close_profile_action();
            return true;
        }
        if !prompt_open && self.worlds.action.is_some() {
            self.worlds.action = None;
            return true;
        }
        let open = prompt_open || self.ui.sheet.is_some();
        if open {
            self.close_sheet();
        }
        open
    }

    /// A same-base native build can still stop for worlds outside of Switch.
    pub(crate) fn held_update(&self) -> Option<Held> {
        let instance = self.selected_instance()?;
        let plays = self.version_of(instance)?;
        let stake = self
            .native_worlds
            .as_ref()
            .filter(|_| self.native_for.as_deref() == Some(instance.id.as_str()))?;
        Some(Held {
            to: build_name(&stake.base),
            plays: build_name(plays),
            mods: stake.mods.clone(),
            details: stake.text.split("\n\n").map(str::to_owned).collect(),
            worlds: stake.worlds.clone(),
            blocked: false,
            base: stake.base.clone(),
            stopped: true,
        })
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

    /// The sheet on screen, if any: a pending question, then the sheet asked for.
    pub(crate) fn live_sheet(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Stateful<Div>> {
        if let Some(sheet) = self.unreviewed_build_sheet(window, cx) {
            return Some(sheet);
        }
        if let Some(pending) = &self.ui.sheets.publisher_change {
            return Some(self.publisher_change_sheet(&pending.question, cx));
        }
        if let Some(pending) = &self.ui.sheets.provider_choice {
            return Some(self.provider_choice_sheet(&pending.need, cx));
        }
        if let Some(body) = self.source_consent_body(cx) {
            return Some(frame("nightly-source-consent", 660., body, cx));
        }
        if let Some((failure, _)) = &self.mods.installs.question {
            return Some(self.registry_build_failed(failure, cx));
        }
        if let Some((impact, _)) = &self.mods.installs.world {
            let action = if impact.removal {
                tr("Remove")
            } else {
                tr("Update")
            };
            let (cancel, approve) = if impact.cancel_default {
                (
                    primary("cancel-registry-change", tr("Cancel")),
                    secondary("approve-registry-change", action),
                )
            } else {
                (
                    secondary("cancel-registry-change", tr("Cancel")),
                    primary("approve-registry-change", action),
                )
            };
            let body = div()
                .flex()
                .flex_col()
                .child(title(impact.heading.clone()))
                .when(!impact.detail.is_empty(), |body| {
                    body.child(
                        div()
                            .id("registry-impact-text")
                            .max_h(px(310.))
                            .overflow_y_scroll()
                            .child(crate::selectable::selectable(impact.detail.clone())),
                    )
                })
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap(px(8.))
                        .child(cancel.on_click(cx.listener(|this, _, _, cx| {
                            this.close_sheet();
                            cx.notify();
                        })))
                        .child(approve.on_click(cx.listener(|this, _, _, cx| {
                            if let Some((_, answer)) = this.mods.installs.world.take() {
                                let _ = answer.send(true);
                            }
                            cx.notify();
                        }))),
                );
            return Some(frame("registry-world-impact", 690., body, cx));
        }
        if let Some(profile_action) = self.profile_action_sheet(cx) {
            return Some(profile_action);
        }
        if let Some(world_action) = self.world_action_sheet(window, cx) {
            return Some(world_action);
        }
        match self.ui.sheet {
            Some(Sheet::Build) => Some(self.build_sheet(cx)),
            Some(Sheet::Notice) => Some(self.notice_sheet(cx)),
            Some(Sheet::GithubAdd) => Some(self.github_add_sheet(window, cx)),
            Some(Sheet::Refused) => {
                let sheet = self.refused_sheet(cx);
                if sheet.is_none() {
                    self.ui.sheet = None;
                }
                sheet
            }
            Some(Sheet::ReleaseBuilds) => {
                let sheet = self.release_builds_sheet(cx);
                if sheet.is_none() {
                    self.ui.sheet = None;
                }
                sheet
            }
            Some(Sheet::NightlySourceLog) => self
                .source_log_body(cx)
                .map(|body| frame("nightly-source-log", 690., body, cx)),
            Some(Sheet::Versions) => Some(self.versions_sheet(cx)),
            Some(Sheet::Switching) => Some(self.switch_sheet(cx)),
            Some(Sheet::SwitchProvider) => self.provider_switch_sheet(cx),
            Some(Sheet::BuildChanges) => self.switch_receipt_sheet(cx),
            Some(Sheet::AccountMenu) => self.account_menu(cx),
            Some(Sheet::LauncherUpdate) => self.launcher_update_sheet(cx),
            Some(Sheet::ProfileMenu) => Some(self.profile_menu(cx)),
            Some(Sheet::Search) => Some(self.command_search(cx)),
            Some(Sheet::RegistryPlay) => match self.registry_stage() {
                Some(stage) => {
                    use crate::eta::{stage_fraction, stage_label};
                    let body = div()
                        .flex()
                        .flex_col()
                        .child(title(tr("Getting ready to play").into()))
                        .child(muted(crate::eta::with_eta(
                            stage_label(stage),
                            self.registry_eta(),
                        )))
                        .children(
                            stage_fraction(stage)
                                .map(|f| crate::widgets::feedback::progress_bar(Some(f))),
                        )
                        .child(div().flex().justify_end().child(
                            secondary("hide-registry-play", tr("Keep browsing")).on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.close_sheet();
                                    cx.notify();
                                }),
                            ),
                        ));
                    Some(frame("registry-play", 550., body, cx))
                }
                None => {
                    self.ui.sheet = None;
                    None
                }
            },
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_build_menu_marks_nightly_while_a_stable_profile_waits_for_stable() {
        use instances::Channel;
        let release = |tag: &str, nightly: bool| backend::Release {
            tag: tag.into(),
            name: String::new(),
            body: String::new(),
            date: String::new(),
            time: String::new(),
            published_at: String::new(),
            asset: Some(("sunrise.tar.gz".into(), 1)),
            prerelease: false,
            nightly: nightly.then(|| "nightly".into()),
        };
        let mut releases = vec![release("nightly-2026-11-20-aaaaaaa", true)];
        let marked = |releases: &[backend::Release]| {
            let waits = Channel::Stable.builds(releases).next().is_none();
            following_now(Channel::Stable, waits)
        };
        // The marked row is the channel whose build Play starts.
        let plays = instances::play_target(Channel::Stable, &releases, &[], None, None);
        assert_eq!(marked(&releases), Channel::Nightly);
        assert_eq!(
            plays.and_then(|tag| Channel::of_build(tag)),
            Some(Channel::Nightly)
        );
        releases.push(release("26.3-0.1.0", false));
        let plays = instances::play_target(Channel::Stable, &releases, &[], None, None);
        assert_eq!(marked(&releases), Channel::Stable);
        assert_eq!(
            plays.and_then(|tag| Channel::of_build(tag)),
            Some(Channel::Stable)
        );
        // Other channels are marked as saved.
        assert_eq!(following_now(Channel::Dev, false), Channel::Dev);
    }

    #[test]
    fn every_build_menu_error_offers_details_with_the_full_message() {
        let echoed = format!(
            "instances.json: invalid type: string \"{}\", expected u64 at line 1 column 412",
            "W".repeat(26)
        );
        assert_eq!(echoed.chars().count(), 100);
        for error in [
            echoed,
            "Wait for the current operation to finish.".to_owned(),
        ] {
            let mut opened = None;
            let _ = build_menu_error(error.clone(), |whole| {
                opened = Some(whole);
                secondary("alt-build-error-details", tr("Details"))
            });
            assert_eq!(opened.as_deref(), Some(error.as_str()));
        }
    }

    #[test]
    fn build_labels_name_the_channel_and_date_or_version() {
        assert_eq!(build_name("nightly-2026-11-17-abcdef0"), "Nightly · Nov 17");
        assert_eq!(build_name("26.3-0.1.0"), "Stable · 0.1.0");
        assert_eq!(build_name("dev-2026-11-17-abcdef0"), "Dev · Nov 17");
        assert_eq!(build_name("v1.0.0"), "v1.0.0");
    }
}
