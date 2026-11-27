//! The owner gate in the window (`sunrise_launcher::account::launch`): the screen that asks for a
//! Microsoft account that owns Minecraft: Java Edition. It covers the launcher once for players
//! without one (those who played offline before the gate), and again whenever Play is refused.
//! The sign-in runs on it, code and all.
use super::*;
use crate::frame::Reach as _;
use crate::paint::palette;
use crate::widgets::button::{primary, secondary, Look, Size, Style};
use gpui::rgb;

impl Launcher {
    /// Opens the screen and starts the sign-in on it; a sign-in already under way (its code on
    /// screen, or Microsoft still answering) is shown as it is, not started over.
    pub(crate) fn sign_in_to_play(&mut self) {
        self.sign_in_screen.open = true;
        if !self.sign_in_under_way() {
            self.start_sign_in();
        }
    }

    /// The screen's own sign-in button. After a failed Sign in again it retries that same
    /// account, so a different Microsoft account is still refused and who plays stays as it was.
    fn sign_in_from_screen(&mut self) {
        if self.sign_in_under_way() {
            self.sign_in_screen.open = true;
        } else {
            self.retry_sign_in();
        }
    }

    fn sign_in_under_way(&self) -> bool {
        self.sign_in.is_some() && !matches!(self.sign_in_state, None | Some(SignInState::Done(_)))
    }

    /// Closes it; it comes back with the next refused Play.
    fn close_owner_screen(&mut self) {
        self.cancel_sign_in();
        self.sign_in_screen.open = false;
        if !self.settings.owner_notice_seen {
            self.settings.owner_notice_seen = true;
            self.save_settings();
        }
    }

    /// What the screen is called: adding an account beside the ones here is not signing in to
    /// play.
    fn sign_in_title(&self) -> &'static str {
        crate::account_chip::sign_in_title(
            self.ui.after_sign_in.is_some(),
            self.sign_in_screen.target.is_some(),
            self.accounts.accounts.len(),
        )
    }

    pub(crate) fn owner_screen(&mut self, cx: &mut Context<Self>) -> Stateful<Div> {
        let configured = account::client_id().is_some();
        let state = match &self.sign_in_state {
            _ if !configured => Shown::Error(tr(account::NOT_CONFIGURED)),
            Some(SignInState::Starting) => {
                Shown::Starting(crate::i18n::tr_key("Asking Microsoft…"))
            }
            Some(SignInState::Working(phase)) => Shown::Starting(phase.label()),
            Some(SignInState::Code(code)) => Shown::Code(&code.verification_uri, &code.user_code),
            Some(SignInState::Done(Err(e))) => Shown::Error(e),
            Some(SignInState::Done(Ok(_))) | None => Shown::Idle,
        };
        if crate::glass_ui::on() {
            return self.glass_owner_screen(state, configured, cx);
        }
        let said = |text: SharedString, size: f32, color: u32| {
            div().text_size(px(size)).text_color(rgb(color)).child(text)
        };
        let panel = || {
            div()
                .p(px(16.))
                .rounded(crate::tokens::r(crate::tokens::R_CARD))
                .border_1()
                .border_color(rgb(palette().divider))
                .bg(rgb(palette().background))
        };
        let mut card = div()
            .id("owner-card")
            .flex()
            .flex_col()
            .gap(px(16.))
            // A set width: text measured at it wraps where it is drawn. Never taller or wider
            // than the window: a small one scrolls the sheet.
            .w(px(580.))
            .max_w_full()
            .max_h_full()
            .overflow_y_scroll().track_scroll(&crate::frame::scrolls("sign-in"))
            .p(px(28.))
            .rounded(crate::tokens::r(crate::tokens::R_PANEL))
            .border_1()
            .border_color(rgb(palette().line))
            // Solid, as the other sheets are: the page behind must not read through it.
            .bg(rgb(palette().raised))
            .text_color(rgb(palette().text))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(
                        said(tr("Microsoft account").into(), tokens::SMALL, palette().muted)
                            .font_weight(FontWeight::SEMIBOLD),
                    )
                    .child(crate::sheets::title(tr(self.sign_in_title()).into())),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(said(
                        tr("Sunrise needs a Microsoft account that owns Minecraft: Java Edition, like other launchers.").into(),
                        tokens::BODY,
                        palette().text,
                    ))
                    .child(said(
                        tr("No Minecraft install needed: Sunrise downloads it from Mojang for your account.").into(),
                        tokens::BODY,
                        palette().text,
                    )),
            );
        // What follows is in reading order, which is the order Tab takes: the code and its
        // copy, the link, the other ways to open it, and last the way out.
        card = match state {
            Shown::Code(uri, user_code) => {
                let copy = user_code.to_string();
                let (open_uri, private_uri, link_uri, other_uri) = (
                    uri.to_string(),
                    uri.to_string(),
                    uri.to_string(),
                    uri.to_string(),
                );
                card.child(
                    panel()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .justify_between()
                        .gap(px(12.))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(200.))
                                .flex()
                                .flex_col()
                                .gap(px(4.))
                                .child(said(
                                    trf(
                                        "Open {link} in your browser and enter this code:",
                                        // The page as the player would type it.
                                        &[("link", short_link(uri))],
                                    )
                                    .into(),
                                    tokens::LABEL,
                                    palette().muted,
                                ))
                                .child(
                                    div()
                                        .font_family(theme::MONO)
                                        .font_weight(FontWeight::BOLD)
                                        .text_size(px(tokens::DISPLAY))
                                        .line_height(px(36.))
                                        .child(user_code.to_string()),
                                ),
                        )
                        .child(self.copy_button(CopyButton::Code, tr("Copy code"), copy, cx)),
                )
                .children((!microsoft::has_private_window()).then(|| {
                    said(
                        tr("Open this link in a private window to pick another account.").into(),
                        tokens::LABEL,
                        palette().muted,
                    )
                }))
                .child(
                    // The three ways to open the link, at one height in one row; a long
                    // translation wraps to a second row rather than out of the card.
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap(px(8.))
                        .child(
                            primary("owner-open", tr("Open link"))
                                .on_click(move |_, _, cx| cx.open_url(&open_uri)),
                        )
                        .child(if microsoft::has_private_window() {
                            secondary("owner-private", tr("Open in private window"))
                                .on_click(move |_, _, cx| microsoft::open_private(&private_uri, cx))
                                .into_any_element()
                        } else {
                            self.copy_button(CopyButton::Link, tr("Copy link"), link_uri, cx)
                        })
                        .child(
                            secondary("owner-other", tr("Use a different Microsoft account"))
                                .on_click(move |_, _, cx| {
                                    microsoft::use_different_account(&other_uri, cx)
                                }),
                        ),
                )
                .child(said(
                    tr("Waiting for Microsoft… the window goes on by itself.").into(),
                    tokens::LABEL,
                    palette().muted,
                ))
            }
            Shown::Starting(what) => {
                card.child(panel().child(said(tr(what).into(), tokens::BODY, palette().muted)))
            }
            Shown::Error(e) => card.child(panel().child(said(
                e.to_string().into(),
                tokens::BODY,
                palette().danger,
            ))),
            Shown::Idle => card,
        };
        let again = match state {
            Shown::Code(..) | Shown::Starting(_) => None,
            Shown::Error(_) if !configured => None,
            _ => Some(
                // Microsoft's own button, as Microsoft publishes it (`microsoft`).
                microsoft::sign_in_button("owner-sign-in", &self.theme)
                    .tab_index(0)
                    .seen()
                    .focus_visible(|s| s.shadow(crate::frame::focus_ring(palette().focus)))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.sign_in_from_screen();
                        cx.notify();
                    })),
            ),
        };
        card = card.child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .justify_end()
                .gap(px(8.))
                .children(again)
                .child(
                    secondary("owner-later", tr("Not now")).on_click(cx.listener(
                        |this, _: &ClickEvent, _, cx| {
                            this.close_owner_screen();
                            cx.notify();
                        },
                    )),
                ),
        );
        // The launcher stays behind, dimmed: the sheet is over it, not instead of it. Unlike the
        // other sheets a press outside does not close it: only "Not now" and Escape do.
        div()
            .id("owner-gate")
            .absolute()
            .inset_0()
            .occlude()
            .flex()
            .items_center()
            .justify_center()
            .p(px(16.))
            .bg(palette().scrim())
            .on_scroll(|_, _, cx| cx.stop_propagation())
            .child(card)
    }

    /// The screen in the glass look: a dialog plate over the dimmed, blurred launcher (the
    /// mockup's modals), its Sign in the Account card's white Microsoft button, its code in the
    /// card's inner panel.
    fn glass_owner_screen(
        &self,
        state: Shown,
        configured: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        use crate::glass_ui::{self as ui, white};
        use sunrise_launcher_theme::glass::{ink, radius};
        let panel = || {
            div()
                .mt(px(20.))
                .p(px(20.))
                .rounded(px(crate::glass_ui::rad(radius::POPOVER)))
                .bg(white(0.045))
        };
        let mut card = ui::dialog_plate(radius::MODAL)
            .id("owner-card")
            .occlude()
            .flex()
            .flex_col()
            // A set width: text measured at it wraps where it is drawn.
            .w(px(520.))
            .max_w_full()
            .p(px(32.))
            .child(ui::text(13., FontWeight::MEDIUM, ink::QUIET).child(tr("Microsoft account")))
            .child(
                div()
                    .mt(px(8.))
                    .font_family(sunrise_launcher_theme::glass::DISPLAY_FONT)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(px(28.))
                    .line_height(px(35.))
                    .text_color(white(ink::FULL))
                    .child(tr(self.sign_in_title())),
            )
            .child(
                div()
                    .mt(px(12.))
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(ui::text(14., FontWeight::NORMAL, ink::BODY).child(crate::selectable::selectable(tr(
                        "Sunrise needs a Microsoft account that owns Minecraft: Java Edition, like other launchers.",
                    ))))
                    .child(ui::text(14., FontWeight::NORMAL, ink::BODY).child(crate::selectable::selectable(tr(
                        "No Minecraft install needed: Sunrise downloads it from Mojang for your account.",
)))),
            );
        card = match state {
            Shown::Code(uri, user_code) => {
                let copy = user_code.to_string();
                let (private_uri, link_uri, other_uri) =
                    (uri.to_string(), uri.to_string(), uri.to_string());
                card.child(
                    panel()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(16.))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap(px(4.))
                                .child(ui::text(13., FontWeight::NORMAL, ink::NOTE).child(
                                    crate::selectable::selectable(trf(
                                        "Open {link} in your browser and enter this code:",
                                        &[("link", short_link(uri))],
                                    )),
                                ))
                                .child(
                                    div()
                                        .font_family(theme::MONO)
                                        .font_weight(FontWeight::BOLD)
                                        .text_size(px(30.))
                                        .line_height(px(38.))
                                        .text_color(white(ink::FULL))
                                        .child(user_code.to_string()),
                                ),
                        )
                        .child(
                            crate::widgets::button::button(
                                "owner-copy",
                                tr("Copy code"),
                                Look::new(Style::Ghost, Size::Small),
                            )
                            .on_click(move |_, _, cx| {
                                copy_to_clipboard(cx, gpui::ClipboardItem::new_string(copy.clone()))
                            }),
                        ),
                )
                .child(
                    div()
                        .mt(px(8.))
                        .flex()
                        .flex_col()
                        .items_start()
                        .gap(px(4.))
                        .when(!microsoft::has_private_window(), |col| {
                            col.child(ui::text(13., FontWeight::NORMAL, ink::QUIET).child(
                                crate::selectable::selectable(tr(
                                    "Open this link in a private window to pick another account.",
                                )),
                            ))
                        })
                        .child(
                            div()
                                .flex()
                                .gap(px(8.))
                                .child(if microsoft::has_private_window() {
                                    crate::widgets::button::button(
                                        "owner-private",
                                        tr("Open in private window"),
                                        Look::new(Style::Ghost, Size::Small),
                                    )
                                    .on_click(
                                        move |_, _, cx| microsoft::open_private(&private_uri, cx),
                                    )
                                } else {
                                    crate::widgets::button::button(
                                        "owner-copy-link",
                                        tr("Copy link"),
                                        Look::new(Style::Ghost, Size::Small),
                                    )
                                    .on_click(
                                        move |_, _, cx| {
                                            copy_to_clipboard(
                                                cx,
                                                gpui::ClipboardItem::new_string(link_uri.clone()),
                                            )
                                        },
                                    )
                                })
                                .child(
                                    crate::widgets::button::button(
                                        "owner-other",
                                        tr("Use a different Microsoft account"),
                                        Look::new(Style::Ghost, Size::Small),
                                    )
                                    .on_click(
                                        move |_, _, cx| {
                                            microsoft::use_different_account(&other_uri, cx)
                                        },
                                    ),
                                ),
                        ),
                )
            }
            Shown::Starting(what) => card
                .child(panel().child(ui::text(14., FontWeight::NORMAL, ink::BODY).child(tr(what)))),
            Shown::Error(e) => card.child(
                panel().child(
                    ui::text(14., FontWeight::NORMAL, ink::FULL)
                        .text_color(ui::color(sunrise_launcher_theme::glass::DANGER))
                        .child(e.to_string()),
                ),
            ),
            Shown::Idle => card,
        };
        let main: Option<AnyElement> = match state {
            Shown::Code(uri, _) => {
                let uri = uri.to_string();
                Some(
                    primary("owner-open", tr("Open link"))
                        .h(px(44.))
                        .px(px(24.))
                        .on_click(move |_, _, cx| cx.open_url(&uri))
                        .into_any_element(),
                )
            }
            Shown::Starting(_) => None,
            Shown::Error(_) if !configured => None,
            _ => Some(
                div()
                    .id("owner-sign-in")
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(px(10.))
                    .h(px(44.))
                    .px(px(20.))
                    .rounded(px(crate::glass_ui::rad(radius::ROW)))
                    .bg(gpui::white())
                    .cursor_pointer()
                    .hover(|s| s.opacity(0.9))
                    .font_family(sunrise_launcher_theme::glass::FONT)
                    .text_size(px(15.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(gpui::rgb(sunrise_launcher_theme::glass::ON_LIGHT))
                    // Reachable with Tab, with the focus ring once the keyboard is in use.
                    .tab_index(0)
                    .focus_visible(|s| s.shadow(crate::frame::focus_ring(palette().focus)))
                    .child(microsoft::logo(18.))
                    .child(tr("Sign in with Microsoft"))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.sign_in_from_screen();
                        cx.notify();
                    }))
                    .into_any_element(),
            ),
        };
        let later = crate::widgets::button::button(
            "owner-later",
            tr("Not now"),
            Look::new(Style::Ghost, Size::Medium),
        )
        .h(px(44.))
        .px(px(20.))
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
            this.close_owner_screen();
            cx.notify();
        }));
        card = card.child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .mt(px(24.))
                .children(main)
                .child(later),
        );
        if matches!(state, Shown::Code(..)) {
            card = card.child(
                ui::text(13., FontWeight::NORMAL, ink::QUIET)
                    .mt(px(12.))
                    .child(crate::selectable::selectable(tr(
                        "Waiting for Microsoft… the window goes on by itself.",
                    ))),
            );
        }
        div()
            .id("owner-gate")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .p(px(24.))
            .on_scroll(|_, _, cx| cx.stop_propagation())
            .occlude()
            .child(ui::scrim_backdrop())
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .bg(ui::scrim()),
            )
            .child(card)
    }
}

const COPY_SHOWN: Duration = Duration::from_millis(1400);

#[derive(Clone, Copy)]
enum CopyButton {
    Code,
    Link,
}

impl CopyButton {
    fn id(self) -> &'static str {
        match self {
            Self::Code => "owner-copy",
            Self::Link => "owner-copy-link",
        }
    }
}

/// The two copy indicators for the current sign-in attempt.
#[derive(Default)]
pub(super) struct SignInCopies {
    code: Option<(Instant, usize)>,
    link: Option<(Instant, usize)>,
    serial: usize,
}

impl SignInCopies {
    pub(super) fn clear(&mut self) {
        self.code = None;
        self.link = None;
    }

    fn shown(&self, kind: CopyButton) -> Option<usize> {
        match kind {
            CopyButton::Code => self.code,
            CopyButton::Link => self.link,
        }
        .filter(|(at, _)| at.elapsed() < COPY_SHOWN)
        .map(|(_, serial)| serial)
    }

    fn copy(&mut self, kind: CopyButton) {
        self.serial += 1;
        let mark = Some((Instant::now(), self.serial));
        match kind {
            CopyButton::Code => self.code = mark,
            CopyButton::Link => self.link = mark,
        }
    }
}

impl Launcher {
    fn copy_button(
        &self,
        kind: CopyButton,
        label: &str,
        text: String,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        crate::widgets::motion_bits::copy_face(kind.id(), label, self.sign_in_copies.shown(kind))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                crate::widgets::copy_quietly(cx, gpui::ClipboardItem::new_string(text.clone()));
                this.sign_in_copies.copy(kind);
                cx.notify();
                cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(COPY_SHOWN).await;
                    let _ = this.update(cx, |_, cx| cx.notify());
                })
                .detach();
            }))
            .into_any_element()
    }
}

/// What the sheet shows of the sign-in.
#[derive(Clone, Copy)]
enum Shown<'a> {
    Idle,
    /// What the sign-in is doing.
    Starting(&'static str),
    /// The page and the code to enter there.
    Code(&'a str, &'a str),
    Error(&'a str),
}

/// `https://www.microsoft.com/link` as `microsoft.com/link`.
fn short_link(uri: &str) -> &str {
    let uri = uri
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    uri.strip_prefix("www.").unwrap_or(uri)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_buttons_keep_their_identity_when_feedback_appears_and_resets() {
        let mut copies = SignInCopies::default();
        for kind in [CopyButton::Code, CopyButton::Link] {
            let expected = Some(ElementId::from(kind.id()));
            let mut initial =
                crate::widgets::motion_bits::copy_face(kind.id(), "Copy", copies.shown(kind));
            assert_eq!(initial.interactivity().element_id, expected);

            copies.copy(kind);
            assert!(copies.shown(kind).is_some());
            let mut copied =
                crate::widgets::motion_bits::copy_face(kind.id(), "Copy", copies.shown(kind));
            assert_eq!(copied.interactivity().element_id, expected);

            copies.clear();
            assert!(copies.shown(kind).is_none());
            let mut reset =
                crate::widgets::motion_bits::copy_face(kind.id(), "Copy", copies.shown(kind));
            assert_eq!(reset.interactivity().element_id, expected);
        }

        copies.copy(CopyButton::Code);
        assert!(copies.shown(CopyButton::Link).is_none());
    }

    #[test]
    fn links_are_shown_as_typed() {
        assert_eq!(
            super::short_link("https://www.microsoft.com/link"),
            "microsoft.com/link"
        );
        assert_eq!(
            super::short_link("https://login.live.com/oauth20_remoteconnect.srf"),
            "login.live.com/oauth20_remoteconnect.srf"
        );
    }
}
