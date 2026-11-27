//! Launcher update actions, shared by the top bar, its release-notes sheet and Settings.
use super::*;
use crate::paint::palette;
use crate::sheets::{self, Sheet};
use crate::widgets::button::{primary, secondary};
use gpui::rgb;
use sunrise_launcher::update_handoff;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Status<'a> {
    Available,
    Downloading(f32),
    Discarding,
    Ready,
    Disabled,
    Stuck(Option<&'a str>),
    Failed(&'a str),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Wait {
    Game,
    Work,
}

impl Wait {
    pub(super) fn for_activity(
        game: bool,
        importing_build: bool,
        hosting: &servers_page::Servers,
        other_work: bool,
    ) -> Option<Self> {
        if game {
            Some(Self::Game)
        } else if importing_build || hosting.writing() || other_work {
            Some(Self::Work)
        } else {
            None
        }
    }
}

pub(super) struct Offer<'a> {
    release: &'a Release,
    pub(super) version: &'a str,
    pub(super) status: Status<'a>,
    dot: bool,
}

impl<'a> Offer<'a> {
    fn new(
        state: &'a SelfUpdate,
        found: Option<&'a Release>,
        dismissed: Option<&str>,
        allowed: bool,
        managed: bool,
    ) -> Option<Self> {
        let (release, status) = match state {
            SelfUpdate::Idle => (found?, Status::Available),
            SelfUpdate::Downloading(release, progress) => (release, Status::Downloading(*progress)),
            SelfUpdate::Ready(release) => (release, Status::Ready),
            SelfUpdate::Discarding => (found?, Status::Discarding),
            SelfUpdate::Stuck(error) => (found?, Status::Stuck(error.as_deref())),
            SelfUpdate::Failed(release, error) => (release, Status::Failed(error)),
        };
        let version = backend::launcher_version(&release.tag)?;
        let status = if managed {
            Status::Stuck(None)
        } else if !allowed {
            Status::Disabled
        } else {
            status
        };
        Some(Self {
            release,
            version,
            status,
            dot: dismissed == Some(version),
        })
    }

    pub(super) fn action(&self, wait: Option<Wait>, armed: bool) -> Option<String> {
        if armed
            || !matches!(
                self.status,
                Status::Available | Status::Ready | Status::Failed(_)
            )
        {
            return None;
        }
        Some(match wait {
            Some(Wait::Game) => tr("Update when you close the game").into(),
            Some(Wait::Work) => tr("Update when the current task finishes").into(),
            None => match self.status {
                Status::Ready => tr("Restart to update").into(),
                Status::Failed(_) => tr("Try update again").into(),
                _ => trf("Update to {version}", &[("version", self.version)]),
            },
        })
    }

    pub(super) fn about(&self, launcher: &Launcher) -> String {
        match self.status {
            Status::Downloading(progress) => trf(
                "Downloading {version} · {percent}%",
                &[
                    ("version", self.version),
                    ("percent", &format!("{:.0}", progress * 100.)),
                ],
            ),
            _ if launcher.restart_for_launcher_update => match launcher.launcher_update_wait() {
                Some(Wait::Game) => tr("The launcher will update when you close the game.").into(),
                Some(Wait::Work) => {
                    tr("The launcher will update when the current task finishes.").into()
                }
                None => tr("Ready to restart").into(),
            },
            Status::Ready => tr("Ready to restart").into(),
            Status::Discarding => tr("Finishing the previous download…").into(),
            Status::Stuck(error) => launcher.launcher_update_help(error),
            Status::Failed(error) => trf("the update failed: {error}", &[("error", error)]),
            Status::Disabled => tr("Automatic launcher updates are disabled.").into(),
            Status::Available => trf("{version} available", &[("version", self.version)]),
        }
    }
}

impl SelfUpdate {
    pub(super) fn ready_to_restart(
        &self,
        armed: bool,
        allowed: bool,
        wait: impl FnOnce() -> Option<Wait>,
    ) -> Option<&Release> {
        match self {
            Self::Ready(release) if armed && allowed && wait().is_none() => Some(release),
            _ => None,
        }
    }
}

impl SelfUpdate {
    /// Keep an attempt pinned while the download or the player's request is still in flight.
    fn reconcile_offer(
        &mut self,
        latest: Option<&Release>,
        armed: bool,
        drop_stage: impl FnOnce(),
    ) {
        match self {
            Self::Failed(attempt, _) if latest.is_none_or(|release| release.tag != attempt.tag) => {
                *self = Self::Idle;
            }
            Self::Ready(staged)
                if !armed
                    && latest.is_some_and(|release| {
                        backend::compare_versions(&release.tag, &staged.tag).is_gt()
                    }) =>
            {
                drop_stage();
                *self = Self::Idle;
            }
            _ => {}
        }
    }

    /// A fetch error doesn't revoke trust, but a rejected manifest does. A download already
    /// writing its stage keeps the slot until its worker reports completion.
    fn withdraw(&mut self, tag: &str, drop_stage: impl FnOnce()) -> bool {
        let next = match self {
            Self::Downloading(release, _) if release.tag == tag => Self::Discarding,
            Self::Ready(release) if release.tag == tag => {
                drop_stage();
                Self::Idle
            }
            Self::Failed(release, _) if release.tag == tag => Self::Idle,
            _ => return false,
        };
        *self = next;
        true
    }

    fn check_failed(
        &mut self,
        updates: &mut Updates,
        armed: &mut bool,
        error: backend::LauncherCheckError,
    ) {
        if let backend::LauncherCheckError::Rejected { tag, .. } = &error {
            if updates
                .launcher
                .as_ref()
                .is_some_and(|release| release.tag == *tag)
            {
                updates.launcher = None;
            }
            if self.withdraw(tag, update_handoff::forget) {
                *armed = false;
            }
        }
        settings::log_launcher_check_failure(error.message(), |line| eprintln!("{line}"));
        updates.launcher_error = Some(error);
    }

    /// Settle the current worker before reconciling against the latest offer.
    fn finish_download(
        &mut self,
        updates: &mut Updates,
        armed: &mut bool,
        result: Result<Option<backend::Staged>, backend::StageUpdateError>,
    ) -> bool {
        if matches!(self, Self::Discarding) {
            if let Ok(Some(staged)) = result {
                backend::remove_stage(&backend::update_work_dir(), &staged.payload);
            }
            *self = Self::Idle;
            return true;
        }
        let Self::Downloading(release, _) = self else {
            return false;
        };
        match result {
            Ok(staged) => {
                update_handoff::set_pending(staged);
                *self = Self::Ready(release.clone());
            }
            Err(backend::StageUpdateError::Check(
                error @ backend::LauncherCheckError::Rejected { .. },
            )) => {
                // This worker has already finished, unlike a rejected check during a download.
                *armed = false;
                *self = Self::Idle;
                self.check_failed(updates, armed, error);
            }
            Err(error) => {
                *armed = false;
                *self = Self::Failed(release.clone(), error.into());
            }
        }
        true
    }
}

impl Launcher {
    pub(super) fn note_launcher_check(
        &mut self,
        result: Result<Option<Release>, backend::LauncherCheckError>,
    ) {
        match result {
            Ok(release) => {
                self.updates.launcher = release;
                self.updates.launcher_error = None;
            }
            Err(error) => self.self_update.check_failed(
                &mut self.updates,
                &mut self.restart_for_launcher_update,
                error,
            ),
        }
        self.reconcile_launcher_offer();
    }

    pub(super) fn cancel_launcher_restart(&mut self) {
        self.restart_for_launcher_update = false;
        self.reconcile_launcher_offer();
    }

    fn reconcile_launcher_offer(&mut self) {
        let found = self.updates.launcher.as_ref();
        self.self_update.reconcile_offer(
            found,
            self.restart_for_launcher_update,
            update_handoff::forget,
        );
        if found.is_some()
            && matches!(self.self_update, SelfUpdate::Idle)
            && backend::self_update_allowed()
            && !backend::package_managed_launcher()
            && backend::cannot_replace_itself()
        {
            self.self_update = SelfUpdate::Stuck(None);
        }
        if self.ui.sheet == Some(Sheet::LauncherUpdate) && self.launcher_update_offer().is_none() {
            // No sheet left to close: don't answer any other dialog on top of it.
            self.ui.sheet = None;
        }
    }

    pub(super) fn launcher_update_offer(&self) -> Option<Offer<'_>> {
        Offer::new(
            &self.self_update,
            self.updates.launcher.as_ref(),
            self.settings.dismissed_launcher_update.as_deref(),
            backend::self_update_allowed(),
            backend::package_managed_launcher(),
        )
    }

    pub(super) fn launcher_update_wait(&self) -> Option<Wait> {
        Wait::for_activity(
            self.client.is_some() || self.starting_client.is_some() || self.starting_pack.is_some(),
            self.importing_build,
            &self.servers,
            self.launch_busy()
                || self
                    .instances
                    .dialog
                    .as_ref()
                    .is_some_and(|dialog| dialog.busy)
                || !self.removing.is_empty()
                || matches!(
                    self.ui.sheets.profile_action,
                    Some(
                        profile_delete::Action::Deleting(_)
                            | profile_delete::Action::Problem { .. }
                            | profile_delete::Action::Interrupted(_)
                    )
                )
                || self.settings_save_error.is_some()
                || self.storage_pending
                || self.finishing_game > 0
                || !self.auth_busy.is_empty()
                || (self.sign_in.is_some()
                    && !matches!(self.sign_in_state, Some(SignInState::Done(_))))
                || self.skins.writing(),
        )
    }

    pub(super) fn request_launcher_update(&mut self) {
        let Some(offer) = self.launcher_update_offer() else {
            return;
        };
        match offer.status {
            Status::Available | Status::Failed(_) => {
                if backend::cannot_replace_itself() {
                    self.self_update = SelfUpdate::Stuck(None);
                    self.open_sheet(Sheet::LauncherUpdate);
                    return;
                }
                let release = offer.release.clone();
                self.self_update = SelfUpdate::Downloading(release.clone(), 0.);
                let tx = self.tx.clone();
                std::thread::spawn(move || {
                    let progress = |p| {
                        let _ = tx.send(Msg::LauncherProgress(p));
                    };
                    let _ = tx.send(Msg::LauncherUpdated(backend::stage_update(
                        &release, progress,
                    )));
                });
            }
            Status::Ready => {}
            Status::Downloading(_) | Status::Discarding | Status::Disabled | Status::Stuck(_) => {
                return
            }
        }
        self.restart_for_launcher_update = true;
    }

    pub(super) fn launcher_update_finished(
        &mut self,
        result: Result<Option<backend::Staged>, backend::StageUpdateError>,
    ) {
        if self.self_update.finish_download(
            &mut self.updates,
            &mut self.restart_for_launcher_update,
            result,
        ) {
            self.reconcile_launcher_offer();
        }
    }

    /// Update-check messages routed from `launcher_messages::drain`.
    pub(super) fn on_update_message(&mut self, msg: Msg) {
        match msg {
            Msg::Updates(updates) => {
                self.checking_updates = false;
                self.launcher_checked_now = None;
                let updates = *updates;
                self.updates.sunrise = updates.sunrise;
                self.updates.sunrise_error = updates.sunrise_error;
                self.note_launcher_check(updates.launcher_error.map_or(Ok(updates.launcher), Err));
            }
            Msg::LauncherChecked(result) => {
                self.checking_updates = false;
                self.launcher_checked_now = result.is_ok().then(Instant::now);
                self.note_launcher_check(*result);
            }
            Msg::LauncherProgress(p) => {
                if let SelfUpdate::Downloading(_, f) = &mut self.self_update {
                    *f = p;
                }
            }
            Msg::LauncherUpdated(result) => self.launcher_update_finished(result),
            _ => unreachable!("launcher_messages::drain only routes update messages here"),
        }
    }

    /// Only the player's update action arms a restart. Closing the window still drops the stage.
    pub(super) fn restart_launcher_if_ready(&mut self, cx: &mut Context<Self>) {
        self.try_restart_launcher(backend::self_update_allowed(), cx, |cx| {
            if update_handoff::start_now()? {
                servers_page::keep_running_through_update();
                shut_down(cx);
                Ok(())
            } else {
                Err(backend::restart())
            }
        });
    }

    pub(super) fn try_restart_launcher(
        &mut self,
        allowed: bool,
        cx: &mut Context<Self>,
        hand_off: impl FnOnce(&mut Context<Self>) -> Result<(), String>,
    ) {
        let Some(release) =
            self.self_update
                .ready_to_restart(self.restart_for_launcher_update, allowed, || {
                    self.launcher_update_wait()
                })
        else {
            return;
        };
        self.restart_for_launcher_update = false;
        if let Err(error) = hand_off(cx) {
            self.self_update = SelfUpdate::Failed(release.clone(), error);
            // This can run on a tick with no messages, just after an Update press.
            cx.notify();
        }
    }

    pub(super) fn launcher_update_control(&self, cx: &mut Context<Self>) -> Option<Div> {
        let offer = self.launcher_update_offer()?;
        let p = palette();
        let wait = self.launcher_update_wait();
        let action = offer.action(wait, self.restart_for_launcher_update);
        let available = trf("Update available: {version}", &[("version", offer.version)]);
        let open = cx.listener(|this, _, _, cx| {
            this.open_sheet(Sheet::LauncherUpdate);
            cx.notify();
        });
        if offer.dot {
            return Some(
                div().flex_none().child(
                    secondary("launcher-update-dot", "")
                        .size(px(tokens::FIELD))
                        .px_0()
                        .tooltip(crate::glass_ui::tip(available))
                        .child(div().size(px(7.)).rounded_full().bg(rgb(p.accent)))
                        .on_click(open),
                ),
            );
        }
        let label = match (&action, wait, offer.status) {
            (Some(_), Some(Wait::Game), _) => tr("Update after game").into(),
            (Some(_), Some(Wait::Work), _) => tr("Update after task").into(),
            (Some(label), _, _) => label.clone(),
            (None, _, Status::Downloading(progress)) => trf(
                "Downloading · {percent}%",
                &[("percent", &format!("{:.0}", progress * 100.))],
            ),
            (None, _, Status::Ready) => tr("Update queued").into(),
            _ => trf("{version} available", &[("version", offer.version)]),
        };
        let tip = action.clone().unwrap_or_else(|| {
            if matches!(offer.status, Status::Disabled | Status::Stuck(_)) {
                tr("What's new").into()
            } else {
                label.clone()
            }
        });
        let main = secondary("launcher-update", "")
            .max_w(px(172.))
            .overflow_hidden()
            .h(px(tokens::FIELD))
            .gap(px(6.))
            .tooltip(crate::glass_ui::tip(tip))
            .child(div().min_w_0().truncate().child(label))
            .when(action.is_none(), |b| {
                b.child(
                    crate::glass_ui::icon("information-circle", 16., rgb(p.chrome_muted).into())
                        .flex_none(),
                )
            })
            .when(action.is_some(), |b| {
                b.border_color(rgb(p.accent)).text_color(rgb(p.chrome_text))
            });
        let main = if action.is_some() {
            main.on_click(cx.listener(|this, _, _, cx| {
                this.request_launcher_update();
                cx.notify();
            }))
        } else {
            main.on_click(open)
        };
        Some(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(4.))
                .child(main)
                .when(action.is_some(), |row| {
                    row.child(
                        secondary("launcher-update-notes", "")
                            .size(px(tokens::FIELD))
                            .px_0()
                            .tooltip(crate::glass_ui::tip(tr("What's new")))
                            .child(crate::glass_ui::icon(
                                "information-circle",
                                16.,
                                rgb(p.chrome_muted).into(),
                            ))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.open_sheet(Sheet::LauncherUpdate);
                                cx.notify();
                            })),
                    )
                }),
        )
    }

    pub(super) fn launcher_update_sheet(&self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let offer = self.launcher_update_offer()?;
        let title = trf("Sunrise Launcher {version}", &[("version", offer.version)]);
        let current = trf(
            "Installed version: {version}",
            &[("version", backend::VERSION)],
        );
        let notes = backend::notes_summary(&offer.release.body);
        let url = format!(
            "https://github.com/{}/releases/tag/{}",
            backend::LAUNCHER_REPO,
            offer.release.tag
        );
        let status = offer.about(self);
        let action = offer.action(
            self.launcher_update_wait(),
            self.restart_for_launcher_update,
        );
        let dismiss = (!offer.dot).then(|| {
            let version = offer.version.to_owned();
            secondary("launcher-update-hide", tr("Hide for this version")).on_click(cx.listener(
                move |this, _, _, cx| {
                    this.settings.dismissed_launcher_update = Some(version.clone());
                    this.save_settings();
                    this.close_sheet();
                    cx.notify();
                },
            ))
        });
        let cancel = self.restart_for_launcher_update.then(|| {
            secondary("launcher-update-cancel-restart", tr("Cancel restart")).on_click(cx.listener(
                |this, _, _, cx| {
                    this.cancel_launcher_restart();
                    cx.notify();
                },
            ))
        });
        let p = palette();
        let body = div()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .child(sheets::title(title))
                    .child(
                        div()
                            .text_size(px(tokens::LABEL))
                            .text_color(rgb(p.muted))
                            .child(current),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .items_start()
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(tr("What's new")),
                    )
                    .children(notes.map(crate::selectable::selectable))
                    .child(
                        secondary("launcher-release-notes", tr("Read the full release notes"))
                            .on_click(move |_, _, cx| cx.open_url(&url)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .pt(px(14.))
                    .border_t_1()
                    .border_color(rgb(p.line))
                    .child(
                        div()
                            .text_size(px(tokens::LABEL))
                            .text_color(rgb(p.muted))
                            .child(crate::selectable::selectable(status)),
                    )
                    .children(match offer.status {
                        Status::Downloading(progress) => {
                            Some(crate::widgets::feedback::progress_bar(Some(progress)))
                        }
                        _ => None,
                    }),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .justify_end()
                    .gap(px(8.))
                    .children(dismiss)
                    .child(
                        secondary(
                            "launcher-update-later",
                            if self.restart_for_launcher_update {
                                tr("Close")
                            } else {
                                tr("Later")
                            },
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.close_sheet();
                            cx.notify();
                        })),
                    )
                    .children(cancel)
                    .children(action.map(|label| {
                        primary("launcher-update-confirm", label).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.request_launcher_update();
                                cx.notify();
                            },
                        ))
                    })),
            );
        Some(sheets::frame("launcher-update-sheet", 520., body, cx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str) -> Release {
        Release {
            tag: tag.into(),
            name: String::new(),
            body: "A shorter route to your installed packs.".into(),
            date: String::new(),
            time: String::new(),
            published_at: String::new(),
            asset: None,
            prerelease: false,
            nightly: None,
        }
    }

    #[test]
    fn shows_a_discovered_update_and_keeps_showing_it_through_download_and_restart() {
        assert!(Offer::new(&SelfUpdate::Idle, None, None, true, false).is_none());
        let found = release("v1.3.0");
        let offer = Offer::new(&SelfUpdate::Idle, Some(&found), None, true, false).unwrap();
        assert_eq!(offer.status, Status::Available);
        assert_eq!(
            offer.action(None, false).as_deref(),
            Some("Update to 1.3.0")
        );
        assert!(!offer.dot);
        // A refresh cannot strip an active update of its version or notes.
        let downloading = SelfUpdate::Downloading(found.clone(), 0.5);
        let offer = Offer::new(&downloading, None, None, true, false).unwrap();
        assert_eq!(offer.version, "1.3.0");
        assert_eq!(offer.status, Status::Downloading(0.5));
        assert!(offer.action(None, true).is_none());
        let ready = SelfUpdate::Ready(found);
        let offer = Offer::new(&ready, None, None, true, false).unwrap();
        assert_eq!(offer.status, Status::Ready);
        assert_eq!(
            offer.action(None, false).as_deref(),
            Some("Restart to update")
        );
    }

    #[test]
    fn dismissal_survives_reopening_but_only_for_the_same_version() {
        let saved = serde_json::to_string(&Settings {
            dismissed_launcher_update: Some("1.3.0".into()),
            ..Settings::default()
        })
        .unwrap();
        let settings = Settings::parse(&saved);
        let dismissed = settings.dismissed_launcher_update.as_deref();
        for (tag, dot) in [("v1.3.0", true), ("v1.3.1", false)] {
            let found = release(tag);
            let offer =
                Offer::new(&SelfUpdate::Idle, Some(&found), dismissed, true, false).unwrap();
            assert_eq!(offer.dot, dot);
        }
        assert!(Settings::parse("{}").dismissed_launcher_update.is_none());
        let malformed = Settings::parse(r#"{"dismissed_launcher_update":42,"skip_opening":true}"#);
        assert!(malformed.dismissed_launcher_update.is_none());
        assert!(malformed.skip_opening);
    }

    #[test]
    fn disabled_self_update_and_managed_installs_show_notes_but_no_install_action() {
        let found = release("v1.3.0");
        let offer = Offer::new(&SelfUpdate::Idle, Some(&found), None, false, false).unwrap();
        assert_eq!(offer.status, Status::Disabled);
        assert!(offer.action(None, false).is_none());
        assert_eq!(offer.version, "1.3.0");
        assert_eq!(offer.release.body, found.body);
        for allowed in [true, false] {
            let offer = Offer::new(&SelfUpdate::Idle, Some(&found), None, allowed, true).unwrap();
            assert_eq!(offer.status, Status::Stuck(None));
            assert!(offer.action(None, false).is_none());
        }
    }

    #[test]
    fn a_download_that_finishes_during_a_write_waits_for_its_worker() {
        let hosting = servers_page::Servers::new();
        for (importing, other_work) in [(true, false), (false, true)] {
            let ready = SelfUpdate::Ready(release("v1.3.0"));
            let wait = Wait::for_activity(false, importing, &hosting, other_work);
            assert_eq!(wait, Some(Wait::Work));
            assert!(ready.ready_to_restart(true, true, || wait).is_none());
            assert!(ready
                .ready_to_restart(true, true, || Wait::for_activity(
                    false, false, &hosting, false
                ))
                .is_some());
        }
    }

    #[test]
    fn canceling_a_restart_exposes_the_newer_verified_release_without_another_check() {
        let old = release("v1.3.0");
        let latest = release("v1.3.1");
        for canceled_before_completion in [true, false] {
            let mut state = SelfUpdate::Downloading(old.clone(), 0.5);
            state.reconcile_offer(Some(&latest), !canceled_before_completion, || {
                panic!("the download still owns its stage")
            });
            assert!(
                Offer::new(&state, Some(&latest), Some("1.3.0"), true, false)
                    .unwrap()
                    .dot
            );
            state = SelfUpdate::Ready(old.clone());
            if !canceled_before_completion {
                state.reconcile_offer(Some(&latest), true, || panic!("the restart is still armed"));
                assert!(
                    Offer::new(&state, Some(&latest), Some("1.3.0"), true, false)
                        .unwrap()
                        .dot
                );
            }
            let mut dropped = false;
            state.reconcile_offer(Some(&latest), false, || dropped = true);
            assert!(dropped);
            let offer = Offer::new(&state, Some(&latest), Some("1.3.0"), true, false).unwrap();
            assert_eq!(offer.version, "1.3.1");
            assert!(!offer.dot);
        }
    }

    #[test]
    fn a_rejected_manifest_invalidates_only_its_attempt_and_never_restarts_it() {
        let found = release("v1.3.0");
        let mut staged = SelfUpdate::Ready(found.clone());
        assert!(!staged.withdraw("v1.3.1", || panic!("unrelated tag")));
        let mut dropped = false;
        assert!(staged.withdraw(&found.tag, || dropped = true));
        assert!(dropped);
        assert!(staged.ready_to_restart(true, true, || None).is_none());
        let mut downloading = SelfUpdate::Downloading(found.clone(), 0.5);
        assert!(downloading.withdraw(&found.tag, || panic!("stage is still being written")));
        assert!(Offer::new(&downloading, None, None, true, false).is_none());
        let fresh = release("v1.3.1");
        let offer = Offer::new(&downloading, Some(&fresh), None, true, false).unwrap();
        assert_eq!(offer.version, "1.3.1");
        assert!(offer.action(None, false).is_none());
    }

    #[test]
    fn a_stage_rejection_withdraws_only_its_tag_but_a_transport_failure_can_retry() {
        use backend::{LauncherCheckError, StageUpdateError};
        let release = release("v1.3.0");
        for (offer, transport) in [("v1.3.0", false), ("v1.3.1", false), ("v1.3.0", true)] {
            let mut updates = Updates {
                launcher: Some(Release {
                    tag: offer.into(),
                    ..release.clone()
                }),
                ..Updates::default()
            };
            let mut armed = true;
            let mut attempt = SelfUpdate::Downloading(release.clone(), 0.5);
            let error = if transport {
                LauncherCheckError::Unavailable("offline".into())
            } else {
                LauncherCheckError::Rejected {
                    tag: release.tag.clone(),
                    reason: "manifest disappeared".into(),
                }
            };
            assert!(attempt.finish_download(
                &mut updates,
                &mut armed,
                Err(StageUpdateError::Check(error.clone()))
            ));
            assert!(!armed);
            if transport {
                assert!(
                    matches!(attempt, SelfUpdate::Failed(_, ref message) if message == "offline")
                );
                assert!(updates.launcher_error.is_none());
                let offer =
                    Offer::new(&attempt, updates.launcher.as_ref(), None, true, false).unwrap();
                assert_eq!(
                    offer.action(None, false).as_deref(),
                    Some("Try update again")
                );
            } else {
                assert!(matches!(attempt, SelfUpdate::Idle));
                assert_eq!(updates.launcher_error, Some(error));
                let visible = Offer::new(&attempt, updates.launcher.as_ref(), None, true, false);
                assert_eq!(
                    visible.map(|offer| offer.release.tag.as_str()),
                    (offer != release.tag).then_some(offer)
                );
            }
        }
    }

    #[test]
    fn an_armed_update_restarts_only_after_verification_and_when_idle() {
        let found = release("v1.3.0");
        let offer = Offer::new(&SelfUpdate::Idle, Some(&found), None, true, false).unwrap();
        assert_eq!(
            offer.action(Some(Wait::Game), false).as_deref(),
            Some("Update when you close the game")
        );
        assert!(SelfUpdate::Downloading(found.clone(), 1.0)
            .ready_to_restart(true, true, || None)
            .is_none());
        let ready = SelfUpdate::Ready(found);
        assert!(ready
            .ready_to_restart(true, true, || Some(Wait::Game))
            .is_none());
        assert!(ready.ready_to_restart(false, true, || None).is_none());
        assert!(ready.ready_to_restart(true, false, || None).is_none());
        assert!(ready.ready_to_restart(true, true, || None).is_some());
    }
}
