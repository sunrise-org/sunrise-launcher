//! The owner gate in the window (`sunrise_launcher::account::launch`): the screen that asks for a
//! Microsoft account that owns Minecraft: Java Edition. It covers the launcher once for players
//! without one (those who played offline before the gate), and again whenever Play is refused.
//! The sign-in runs on it, code and all.
use super::*;
use crate::frame::Reach as _;
use crate::paint::palette;
use crate::widgets::button::{button, primary, secondary, Look, Size, Style};
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
        let card = div()
            .id("owner-card")
            .flex()
            .flex_col()
            .gap(px(16.))
            // A set width: text measured at it wraps where it is drawn. Never taller or wider
            // than the window: a small one scrolls the sheet.
            .w(px(520.))
            .max_w_full()
            .max_h_full()
            .overflow_y_scroll()
            .track_scroll(&crate::frame::scrolls("sign-in"))
            .p(px(28.))
            .rounded(crate::tokens::r(crate::tokens::R_PANEL))
            .border_1()
            .border_color(rgb(palette().line))
            // Solid, as the other sheets are: the page behind must not read through it.
            .bg(rgb(palette().raised))
            .text_color(rgb(palette().text))
            .child(crate::sheets::title(tr(self.sign_in_title()).into()))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(said(
                        tr("Sunrise needs a Microsoft account that owns Minecraft: Java Edition, like other launchers."),
                        tokens::BODY,
                        palette().text,
                    ))
                    .child(said(
                        tr("No Minecraft install needed: Sunrise downloads it from Mojang for your account."),
                        tokens::BODY,
                        palette().text,
                    )),
            );
        let card = self.owner_actions(card, state, configured, cx);
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

    /// The part of the sheet that follows the sign-in: the code and how to open it, the
    /// progress or the error, Microsoft's button when there is one to press, and the way out.
    fn owner_actions(
        &self,
        card: Stateful<Div>,
        state: Shown<'_>,
        configured: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let code = matches!(state, Shown::Code(..));
        let again = match state {
            Shown::Code(..) | Shown::Starting(_) => None,
            Shown::Error(_) if !configured => None,
            _ => Some(
                // Microsoft's own button, as Microsoft publishes it (`microsoft`).
                microsoft::sign_in_button("owner-sign-in", &self.theme)
                    .w_full()
                    .justify_center()
                    .tab_index(0)
                    .seen()
                    .focus_visible(|s| s.shadow(crate::frame::focus_ring(palette().focus)))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.sign_in_from_screen();
                        cx.notify();
                    })),
            ),
        };
        let card = match state {
            Shown::Code(uri, user_code) => code_part(card, uri, user_code),
            Shown::Starting(what) => {
                card.child(panel().child(said(tr(what), tokens::BODY, palette().muted)))
            }
            Shown::Error(e) => {
                card.child(panel().child(said(e.to_string(), tokens::BODY, palette().danger)))
            }
            Shown::Idle => card,
        };
        // Use a different account sits at the left of the foot while a code is out; the way
        // out at the right.
        let other = match state {
            Shown::Code(uri, _) => {
                let uri = uri.to_string();
                Some(
                    button(
                        "owner-other",
                        tr("Use a different Microsoft account"),
                        Look::new(Style::Ghost, Size::Medium),
                    )
                    .on_click(move |_, _, cx| microsoft::use_different_account(&uri, cx)),
                )
            }
            _ => None,
        };
        let later = secondary(
            "owner-later",
            if code { tr("Cancel") } else { tr("Not now") },
        )
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
            this.close_owner_screen();
            cx.notify();
        }));
        card.children(again).child(
            div()
                .mt(px(8.))
                .flex()
                .flex_wrap()
                .items_center()
                .justify_between()
                .gap(px(8.))
                .child(div().flex().flex_wrap().gap(px(8.)).children(other))
                .child(later),
        )
    }
}

/// The sheet's inner panel: the progress and the error sit in one.
fn panel() -> Div {
    div()
        .p(px(16.))
        .rounded(crate::tokens::r(crate::tokens::R_CARD))
        .border_1()
        .border_color(rgb(palette().divider))
        .bg(rgb(palette().background))
}

/// A paragraph of the sheet at `size` in `color`.
fn said(text: impl Into<SharedString>, size: f32, color: u32) -> Div {
    div()
        .text_size(px(size))
        .text_color(rgb(color))
        .child(text.into())
}

/// The code to type on the page, big in mono, the buttons that open that page, and the note
/// that Microsoft is still answering.
fn code_part(card: Stateful<Div>, uri: &str, user_code: &str) -> Stateful<Div> {
    let (open_uri, private_uri, link_uri) = (uri.to_string(), uri.to_string(), uri.to_string());
    let has_private = microsoft::has_private_window();
    // Copy code, Open link and the third way in one row, in the order Tab takes them.
    let actions = div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(8.))
        .child(crate::widgets::motion_bits::copy_button(
            "owner-copy",
            tr("Copy code"),
            user_code.to_string(),
        ))
        .child(
            primary("owner-open", tr("Open link")).on_click(move |_, _, cx| cx.open_url(&open_uri)),
        )
        .child(if has_private {
            secondary("owner-private", tr("Open in private window"))
                .on_click(move |_, _, cx| microsoft::open_private(&private_uri, cx))
                .into_any_element()
        } else {
            crate::widgets::motion_bits::copy_button("owner-copy-link", tr("Copy link"), link_uri)
                .into_any_element()
        });
    card.child(
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(said(
                trf(
                    "Open {link} in your browser and enter this code:",
                    // The page as the player would type it.
                    &[("link", short_link(uri))],
                ),
                tokens::LABEL,
                palette().muted,
            ))
            .child(code_box(user_code)),
    )
    .child(actions)
    .children((!has_private).then(|| {
        said(
            tr("Open this link in a private window to pick another account."),
            tokens::LABEL,
            palette().muted,
        )
    }))
    .child(said(
        tr("Waiting for Microsoft… the window goes on by itself."),
        tokens::LABEL,
        palette().muted,
    ))
}

/// The code itself: large, in mono, centred on the sheet's sunken field.
fn code_box(user_code: &str) -> Div {
    div()
        .w_full()
        .flex()
        .justify_center()
        .py(px(14.))
        .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
        .border_1()
        .border_color(rgb(palette().line))
        .bg(rgb(palette().field))
        .child(
            div()
                .font_family(theme::MONO)
                .font_weight(FontWeight::BOLD)
                .text_size(px(tokens::DISPLAY))
                .line_height(px(36.))
                .text_color(rgb(palette().text))
                .child(user_code.to_string()),
        )
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
