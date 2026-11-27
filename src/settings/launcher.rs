//! Settings › Launcher: language, network and this version.
use super::*;
use crate::widgets::button::{primary, secondary};
use crate::widgets::dropdown::{Dropdown, Item};
use crate::widgets::feedback::{banner, Tone};

impl Launcher {
    pub(super) fn settings_launcher(&mut self, cx: &mut Context<Self>) -> Div {
        let picking = self.ui.settings.languages;
        let chosen = self.settings.language;
        let system = i18n::Language::system().name();
        // One dropdown with the language on it; 87 languages don't fit on a page as buttons.
        let items: Vec<Item> = std::iter::once(None)
            .chain(i18n::Language::ALL.map(Some))
            .map(|choice| match choice {
                // The first choice stands apart from the languages under it, and says which
                // language it follows.
                None => Item::new(tr("Same as the system"))
                    .sub(system)
                    .current(chosen.is_none())
                    .rule_after(),
                Some(l) => Item::new(l.name()).current(chosen == Some(l)),
            })
            .collect();
        let dropdown = Dropdown::new("alt-language", items, picking)
            .on_toggle({
                let this = cx.entity();
                move |_, cx| {
                    this.update(cx, |this, cx| {
                        this.ui.settings.languages = !this.ui.settings.languages;
                        cx.notify();
                    })
                }
            })
            .on_close({
                let this = cx.entity();
                move |_, cx| {
                    this.update(cx, |this, cx| {
                        this.ui.settings.languages = false;
                        cx.notify();
                    })
                }
            })
            .on_pick({
                let this = cx.entity();
                move |n, _, cx| {
                    this.update(cx, |this, cx| {
                        this.settings.language = match n {
                            0 => None,
                            n => i18n::Language::ALL.get(n - 1).copied(),
                        };
                        i18n::set(this.settings.language());
                        this.save_settings();
                        cx.notify();
                    })
                }
            })
            .build();
        let language = row(tr("Language"), "", dropdown);
        fit_field(&mut self.proxy_input, self.window_width);
        let refused = (!self.proxy_focused)
            .then(|| sunrise_launcher::proxy::refuse(self.proxy_input.text()))
            .flatten();
        let field = self.settings_field(
            "alt-proxy",
            self.proxy_focus.clone(),
            self.proxy_focused,
            &self.proxy_input,
            "For example http://127.0.0.1:7890",
            |this| &mut this.proxy_input,
            |this| this.save_proxy(),
            cx,
        );
        let proxy = self.wide_row(
            tr("Proxy"),
            tr("Downloads, sign-ins and updates go through it. Empty means none; HTTPS_PROXY then applies if the launcher was started with one."),
            field.w(px(340.)),
        );
        let proxy = match refused {
            Some(why) => div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .child(proxy)
                .child(banner(Tone::Danger, why, None)),
            None => proxy,
        };
        let version = self.launcher_update_row(cx);
        div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(group(vec![language]))
            .child(group(vec![proxy]))
            .child(group(vec![version]))
    }

    /// This launcher's version and its own update: the state of the staged download, and the
    /// action that fits it (Update, or Download again when this copy can't replace itself).
    fn launcher_update_row(&mut self, cx: &mut Context<Self>) -> Div {
        let title = trf(
            "Sunrise Launcher {version}",
            &[("version", backend::VERSION)],
        );
        use crate::updates::Status;
        let recovery = || {
            self.launcher_download_available().then(|| {
                let url = backend::download_page(&self.links.website);
                secondary("alt-launcher-download", tr("Download again"))
                    .on_click(move |_, _, cx| cx.open_url(&url))
            })
        };
        let offer = self.launcher_update_offer();
        let retry_download = offer
            .as_ref()
            .is_some_and(|offer| matches!(offer.status, Status::Failed(_)))
            .then(recovery)
            .flatten();
        let rejected = matches!(
            self.updates.launcher_error,
            Some(backend::LauncherCheckError::Rejected { .. })
        );
        let verification_error = || {
            banner(
                Tone::Danger,
                tr("The launcher update could not be verified."),
                None,
            )
        };
        let (about, action, bad) = if let Some(offer) = &offer {
            let armed = self.restart_for_launcher_update;
            let action = if armed {
                Some(
                    secondary("alt-launcher-cancel", tr("Cancel restart")).on_click(cx.listener(
                        |this, _, _, cx| {
                            this.cancel_launcher_restart();
                            cx.notify();
                        },
                    )),
                )
            } else if let Some(label) = offer.action(self.launcher_update_wait(), armed) {
                Some(primary("alt-launcher-update", label).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.request_launcher_update();
                        cx.notify();
                    },
                )))
            } else if matches!(offer.status, Status::Stuck(_)) {
                recovery()
            } else {
                None
            };
            let available = offer.about(self);
            let about = match offer.status {
                Status::Available | Status::Downloading(_) => available,
                _ => trf(
                    "{version} available · {status}",
                    &[("version", offer.version), ("status", &available)],
                ),
            };
            (
                about,
                action,
                matches!(offer.status, Status::Failed(_) | Status::Stuck(Some(_))),
            )
        } else if let SelfUpdate::Stuck(error) = &self.self_update {
            (
                self.launcher_update_help(error.as_deref()),
                recovery(),
                error.is_some(),
            )
        } else if self.checking_updates {
            (
                String::new(),
                Some(off(secondary("alt-launcher-check", tr("Checking…")))),
                false,
            )
        } else {
            let (about, bad) = if rejected {
                (String::new(), true)
            } else {
                launcher_check_line(
                    self.updates
                        .launcher_error
                        .as_ref()
                        .map(backend::LauncherCheckError::message),
                    self.launcher_checked_now,
                    self.checked.is_some(),
                )
            };
            let check = secondary("alt-launcher-check", tr("Check now")).on_click(cx.listener(
                |this, _, _, cx| {
                    this.check_updates(UpdateCheck::ManualLauncher);
                    cx.notify();
                },
            ));
            (about, Some(check), bad)
        };
        let mut words = div().flex().flex_col().gap(px(2.));
        words = words.child(div().font_weight(FontWeight::SEMIBOLD).child(title));
        if !about.is_empty() {
            words = words.child(if bad {
                banner(Tone::Danger, about, None)
            } else {
                hint(about)
            });
        }
        div()
            .flex()
            .items_center()
            .gap(px(16.))
            .child(
                words
                    .flex_1()
                    .min_w_0()
                    .children(rejected.then(verification_error)),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .flex_col()
                    .items_end()
                    .gap(px(6.))
                    .children(action)
                    .children(retry_download),
            )
    }
}
