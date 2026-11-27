//! Settings › Launcher: language, network and this version.
use super::*;
use crate::widgets::button::secondary;
use crate::widgets::feedback::{banner, Tone};

impl Launcher {
    pub(super) fn settings_launcher(&mut self, cx: &mut Context<Self>) -> Div {
        let picking = self.ui.settings.languages;
        let chosen = self.settings.language;
        let system = i18n::Language::system().name();
        let now: SharedString = match chosen {
            Some(l) => l.name().into(),
            None => tr("Same as the system").into(),
        };
        // One button with the language on it; a press opens the list under it, as Discover's
        // menus do. 87 languages don't fit on a page as buttons.
        let opener = secondary("alt-language", now)
            .gap(px(8.))
            .child(crate::widgets::motion_bits::chevron(
                "settings-language",
                picking,
                13.,
                rgb(palette().muted).into(),
            ))
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.ui.settings.languages = !this.ui.settings.languages;
                cx.notify();
            }));
        let menu = picking.then(|| {
            let options = std::iter::once(None)
                .chain(i18n::Language::ALL.map(Some))
                .enumerate()
                .map(|(i, choice)| {
                    let on = choice == chosen;
                    let label: SharedString = match choice {
                        Some(l) => l.name().into(),
                        None => tr("Same as the system").into(),
                    };
                    div()
                        .id(("alt-language-option", i))
                        .reach()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .px(px(10.))
                        .py(px(7.))
                        .rounded(tokens::r(tokens::R_SMALL))
                        .text_size(px(tokens::LABEL))
                        .cursor_pointer()
                        .hover(|s| s.bg(rgb(palette().hover)))
                        .when(on, |d| d.font_weight(FontWeight::SEMIBOLD))
                        .child(div().flex_1().min_w_0().truncate().child(label))
                        // The system's own language, beside the choice that follows it.
                        .when(choice.is_none(), |d| {
                            d.child(
                                div()
                                    .flex_none()
                                    .text_color(rgb(palette().muted))
                                    .child(system),
                            )
                        })
                        .when(on, |d| {
                            d.child(crate::blocks::icon(
                                "check",
                                13.,
                                rgb(palette().text).into(),
                            ))
                        })
                        // The first choice stands apart from the languages under it.
                        .when(choice.is_none(), |d| {
                            d.mb(px(4.)).border_b_1().border_color(rgb(palette().line))
                        })
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.settings.language = choice;
                            i18n::set(this.settings.language());
                            this.save_settings();
                            this.ui.settings.languages = false;
                            cx.notify();
                        }))
                });
            let list = div()
                .id("alt-language-menu")
                .w(px(260.))
                .max_h(px(320.))
                .overflow_y_scroll()
                .p(px(6.))
                .rounded(tokens::r(tokens::R_CONTROL))
                .border_1()
                .border_color(palette().input_edge())
                .bg(rgb(palette().raised))
                .shadow_lg()
                .occlude()
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_scroll(own_wheel)
                .flex()
                .flex_col()
                .children(options);
            // Under the button, right-aligned to it, and kept inside the window: at 800×600 the
            // whole list, edge included, moves up rather than running off the bottom.
            gpui::deferred(
                div()
                    .absolute()
                    .top(px(tokens::CONTROL + 6.))
                    .right_0()
                    .child(
                        gpui::anchored()
                            .anchor(gpui::Anchor::TopRight)
                            .snap_to_window_with_margin(px(8.))
                            .child(list),
                    ),
            )
            .with_priority(10)
        });
        let language = row(
            tr("Language"),
            "",
            div().relative().child(opener).children(menu),
        );
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
        let mut proxy = stacked(
            tr("Proxy"),
            tr("Downloads, sign-ins and updates go through it. Empty means none; HTTPS_PROXY then applies if the launcher was started with one."),
            field,
        );
        if let Some(why) = refused {
            proxy = proxy.child(banner(Tone::Danger, why, None));
        }
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
                Some(
                    secondary("alt-launcher-update", label).on_click(cx.listener(
                        |this, _, _, cx| {
                            this.request_launcher_update();
                            cx.notify();
                        },
                    )),
                )
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
