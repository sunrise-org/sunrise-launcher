//! Settings' two-step data move. Only the separate worker copies data, after this process exits.
use super::*;
use crate::settings::{hint, off, row};
use crate::widgets::button::secondary;
use crate::widgets::feedback::{banner, Tone};

fn env_value(key: &str) -> Option<String> {
    std::env::var_os(key)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string_lossy().into_owned())
}

/// The startup notice and whether the configured data actually failed to open. A failed
/// move still leaves the old folder in use and does not block actions there.
pub(super) fn startup_location() -> (Option<String>, bool) {
    startup_location_status(
        sunrise_launcher::data_location::startup_error().as_deref(),
        env_value("SUNRISE_RELOCATION_ERROR").as_deref(),
    )
}

fn startup_location_status(open: Option<&str>, relocation: Option<&str>) -> (Option<String>, bool) {
    match open {
        Some(error) => (Some(open_error(error)), true),
        None => (
            relocation.map(|error| trf("Data move failed: {error}", &[("error", error)])),
            false,
        ),
    }
}

fn open_error(error: &str) -> String {
    trf(
        "Could not open the data folder: {error}",
        &[("error", error)],
    )
}

pub(super) fn startup_note() -> Option<String> {
    env_value("SUNRISE_RELOCATION_BACKUP").map(|paths| {
        trf(
            "Data moved. Old folders still use disk space: {paths}. Check that your worlds and profiles work in the new location before deleting the old folders.",
            &[("paths", &paths)],
        )
    })
}

fn environment_blocked() -> bool {
    ["SUNRISE_LAUNCHER_HOME", "SUNRISE_HOME"]
        .iter()
        .any(|key| std::env::var_os(key).is_some_and(|value| !value.is_empty()))
}

impl Launcher {
    pub(crate) fn location_busy(&self) -> bool {
        self.installing.is_some()
            || !self.removing.is_empty()
            || self.importing_build
            || self.native_step.is_some()
            || self.preparing
            || self.client.is_some()
            || self.starting_client.is_some()
            || self.starting_pack.is_some()
            || self.registry_stage().is_some()
            || !self.auth_busy.is_empty()
            || (self.sign_in.is_some() && !matches!(self.sign_in_state, Some(SignInState::Done(_))))
            || matches!(
                self.self_update,
                SelfUpdate::Downloading(..) | SelfUpdate::Discarding
            )
            || self.servers.location_busy()
            || self.github.busy()
            || self.instances.dialog.as_ref().is_some_and(|d| d.busy)
            || self.internet.location_busy()
            || self.skins.writing()
    }

    fn location_blocked(&self) -> Option<String> {
        if environment_blocked() {
            Some(tr("Data folders are set by SUNRISE_LAUNCHER_HOME or SUNRISE_HOME. Remove these environment overrides and restart the launcher before moving data here.").into())
        } else if self.location_busy() {
            // A sentence of its own when a skin is being saved, so the other keeps its
            // translations.
            Some(if self.skins.writing() {
                tr("Close the game and servers, and wait for downloads, builds, imports, sign-in and the skin being saved on the Skins tab to finish before moving data.").into()
            } else {
                tr("Close the game and servers, and wait for downloads, builds, imports and sign-in to finish before moving data.").into()
            })
        } else {
            None
        }
    }

    fn choose_location(&mut self, cx: &mut Context<Self>) {
        self.location_error = self.location_blocked();
        if self.location_error.is_some() {
            cx.notify();
            return;
        }
        self.location_target = None;
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(tr("Choose data folder").into()),
        });
        cx.spawn(async move |this, cx| {
            let result = match paths.await {
                Ok(Ok(paths)) => Ok(paths.and_then(|mut paths| paths.pop())),
                Ok(Err(error)) => Err(error.to_string()),
                Err(error) => Err(error.to_string()),
            };
            let _ = this.update(cx, |this, cx| {
                this.location_error = this.location_blocked();
                if this.location_error.is_none() {
                    match result {
                        Ok(Some(path)) => match std::fs::read_dir(&path) {
                            Ok(mut entries) => match entries.next() {
                                None => this.location_target = Some(path),
                                Some(Ok(_)) => {
                                    this.location_error = Some(
                                        tr("Choose an empty folder for Sunrise's data.").into(),
                                    );
                                }
                                Some(Err(error)) => this.location_picker_error(error.to_string()),
                            },
                            Err(error) => this.location_picker_error(error.to_string()),
                        },
                        Ok(None) => {}
                        Err(error) => this.location_picker_error(error),
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Try the configured folder again. A success restarts the launcher, because paths already
    /// resolved to the fallback folders cannot be swapped under a running window.
    pub(crate) fn retry_location(&mut self, cx: &mut Context<Self>) {
        if let Some(wait) = self.restart_blocked_by_skins() {
            self.location_error = Some(wait);
            cx.notify();
            return;
        }
        match sunrise_launcher::data_location::retry() {
            Ok(()) => {
                self.location_error = Some(backend::restart());
            }
            Err(error) => self.location_error = Some(open_error(&error)),
        }
        cx.notify();
    }

    /// Keep the default folders: the pointer is kept beside the old one as `location.json.old`,
    /// and nothing in the moved folders is deleted.
    pub(crate) fn use_default_location(&mut self, cx: &mut Context<Self>) {
        if let Some(wait) = self.restart_blocked_by_skins() {
            self.location_error = Some(wait);
            cx.notify();
            return;
        }
        match sunrise_launcher::data_location::use_default_folder() {
            Ok(()) => {
                self.location_error = Some(backend::restart());
            }
            Err(error) => self.location_error = Some(open_error(&error)),
        }
        cx.notify();
    }

    /// Why the launcher mustn't restart now, if it mustn't: a Save or a saved skin being written
    /// on the Skins tab, for any account, which a restart would cut off. The same check as a data
    /// move's ([`Self::location_busy`]) and the launcher update's restart.
    pub(crate) fn restart_blocked_by_skins(&self) -> Option<String> {
        self.skins.writing().then(|| {
            tr("Wait for the skin being saved on the Skins tab to finish, then try again.").into()
        })
    }

    fn location_picker_error(&mut self, error: String) {
        self.location_error = Some(trf(
            "Could not choose data folder: {error}",
            &[("error", &error)],
        ));
    }

    fn confirm_location(&mut self, cx: &mut Context<Self>) {
        self.location_error = self.location_blocked();
        if self.location_error.is_some() {
            cx.notify();
            return;
        }
        if self.settings_save_error.is_some() && !self.save_settings() {
            cx.notify();
            return;
        }
        let Some(target) = self.location_target.as_ref() else {
            return;
        };
        let result = std::env::current_exe().and_then(|exe| {
            sunrise_launcher::process::command(exe)
                .arg("--relocate-data")
                .arg(target)
                .arg("--restart")
                .spawn()
        });
        match result {
            Ok(_) => {
                // No installer while the worker copies the data folder its stage is in.
                sunrise_launcher::update_handoff::forget();
                cx.quit()
            }
            Err(error) => {
                self.location_error = Some(trf(
                    "Could not start data move: {error}",
                    &[("error", &error.to_string())],
                ));
                cx.notify();
            }
        }
    }

    /// Where the data lives: the folder, Open and Move…, the way to recover when it can't be
    /// opened (Retry, Use the default folder), and the move's confirmation.
    pub(super) fn location_row(&self, cx: &mut Context<Self>) -> Div {
        let blocked = self.location_blocked();
        let move_button = secondary("alt-move-data", tr("Move…"));
        let move_button = if blocked.is_some() {
            off(move_button)
        } else {
            move_button.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.choose_location(cx);
            }))
        };
        let mut body = div().flex().flex_col().gap(px(10.)).child(row(
            tr("Data folder"),
            short(&backend::data_dir()),
            div()
                .child(
                    secondary("alt-open-data", tr("Open folder"))
                        .on_click(|_, _, cx| crate::toast::open(cx, &backend::data_dir())),
                )
                .child(move_button),
        ));
        body = body
            .child(hint(tr(
                "Moves all launcher, game and build data, including settings, worlds, profiles and installed versions. Choose an empty folder with enough free space.",
            )))
            .child(hint(tr(
                "The launcher closes while your data is copied and verified, then reopens. Old folders are kept and still use disk space until you verify the move and remove them yourself.",
            )))
            .children(blocked.map(|t| banner(Tone::Danger, t, None)))
            .children(self.location_note.clone().map(hint))
            .children(self.location_error.clone().map(|t| banner(Tone::Danger, t, None)));
        if sunrise_launcher::data_location::startup_error().is_some() {
            body = body.child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(8.))
                    .child(secondary("alt-retry-data-location", tr("Retry")).on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| this.retry_location(cx)),
                    ))
                    .child(
                        secondary(
                            "alt-use-default-data-location",
                            tr("Use the default folder"),
                        )
                        .on_click(cx.listener(
                            |this, _: &ClickEvent, _, cx| {
                                this.use_default_location(cx);
                            },
                        )),
                    ),
            );
        }
        if let Some(target) = &self.location_target {
            let confirm = secondary("alt-confirm-move-data", tr("Move and restart"));
            let confirm = if self.location_blocked().is_some() {
                off(confirm)
            } else {
                confirm.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.confirm_location(cx);
                }))
            };
            body = body.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(div().child(crate::selectable::selectable(trf(
                        "Move all data to {path}?",
                        &[("path", &target.display().to_string())],
                    ))))
                    .child(div().flex().gap(px(8.)).child(confirm).child(
                        secondary("alt-cancel-move-data", tr("Cancel")).on_click(cx.listener(
                            |this, _: &ClickEvent, _, cx| {
                                this.location_target = None;
                                this.location_error = None;
                                cx.notify();
                            },
                        )),
                    )),
            );
        }
        body
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_move_does_not_block_play_or_profile_switching_but_unavailable_data_does() {
        use crate::launch_state::{location_allows_action, Facts, Main};

        for (open, relocation, available) in [
            (None, None, true),
            (None, Some("disk full"), true),
            (Some("not mounted"), Some("disk full"), false),
        ] {
            let (notice, unavailable) = startup_location_status(open, relocation);
            assert_eq!(!unavailable, available);
            assert_eq!(notice.is_some(), open.is_some() || relocation.is_some());
            if let Some(error) = open.or(relocation) {
                assert!(notice.is_some_and(|notice| notice.contains(error)));
            }
            let play = Facts {
                signed_in: true,
                installed: true,
                located: !unavailable,
                ..Facts::default()
            }
            .view();
            assert_eq!(play.main, Main::Play);
            assert_eq!(play.enabled, available);
            assert_eq!(location_allows_action(!unavailable, false), available);
            assert!(!location_allows_action(!unavailable, true));
        }
    }
}
