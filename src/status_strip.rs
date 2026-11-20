//! The status strip: what Play is doing or what went wrong, with its one action, on every page
//! where Play itself is not in view. Home says the same beside Play, so the strip is drawn
//! there only once Home is scrolled past Play (`HomeState::play_in_view`): never both at once.
//! It is not drawn at all while there is nothing to tell (`launch_state::View::strip`).
use super::*;
use crate::frame::button;
use crate::launch_state::{Act, Main, StripAct};
use crate::paint::palette;
use gpui::rgb;

impl Launcher {
    /// A registry warning can coexist with progress, Stop, or a separate launch error.
    pub(super) fn registry_notice_strip(&self, cx: &mut Context<Self>) -> Option<Div> {
        let line = self.play_registry_notices.first()?.clone();
        Some(
            strip()
                .child(
                    div()
                        .flex_none()
                        .size(px(8.))
                        .rounded_full()
                        .bg(rgb(palette().warning)),
                )
                .child(div().flex_1().min_w_0().child(line))
                .when(self.play_registry_notices.len() > 1, |d| {
                    d.child(
                        button("play-registry-details", tr("Details…")).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.open_notice(this.play_registry_notices.join("\n"), None);
                                cx.notify();
                            },
                        )),
                    )
                })
                .child(
                    button("play-registry-dismiss", tr("Dismiss")).on_click(cx.listener(
                        |this, _, _, cx| {
                            this.play_registry_notices.clear();
                            cx.notify();
                        },
                    )),
                ),
        )
    }

    /// The strip, for the frame to place above the page: one row, as wide as it is given.
    pub(crate) fn status_strip(&mut self, cx: &mut Context<Self>) -> Option<Div> {
        let home = self.ui.page == crate::frame::Page::Home;
        if home {
            if self.ui.home.play_in_view() {
                return None;
            }
        } else {
            self.refresh_home_inventory(cx);
        }
        let view = self.launch_facts().view();
        let Some((says, act)) = view.strip() else {
            // Nothing runs or failed. A build that just ended in the background is said here
            // for a moment, with nothing to press.
            return self.build_said().map(|said| {
                strip()
                    .child(
                        div()
                            .flex_none()
                            .size(px(8.))
                            .rounded_full()
                            .bg(rgb(palette().success)),
                    )
                    .child(div().flex_1().min_w_0().truncate().child(said))
            });
        };
        let id = match act {
            StripAct::Act(Act::StopBuild) => "alt-strip-stop-build",
            StripAct::StopGame => "alt-strip-stop",
            StripAct::Act(_) => "alt-strip-fix",
            StripAct::Home => "alt-strip-home",
        };
        // A build draws its own line: the stage it is at over the three stages' bar.
        let line = match self
            .build_line(true, cx)
            .filter(|_| view.main == Main::Building)
        {
            Some(build) => build.into_any_element(),
            None => div().truncate().child(says).into_any_element(),
        };
        let mark = if view.failed {
            palette().warning
        } else {
            palette().accent
        };
        // A download says how far it is in the label; its bar shows it too.
        let bar = match view.main {
            Main::Installing(progress) => Some(
                div()
                    .flex_none()
                    .w(px(120.))
                    .h(px(4.))
                    .rounded(crate::tokens::r(crate::tokens::R_BAR))
                    .bg(rgb(palette().line))
                    .overflow_hidden()
                    .child(
                        div()
                            .h(px(4.))
                            .rounded(crate::tokens::r(crate::tokens::R_BAR))
                            .bg(rgb(palette().accent))
                            .w(relative(progress.clamp(0.03, 1.))),
                    ),
            ),
            _ => None,
        };
        Some(
            strip()
                .child(div().flex_none().size(px(8.)).rounded_full().bg(rgb(mark)))
                .child(div().flex_1().min_w_0().child(line))
                .children(bar)
                .child(
                    button(id, act.label()).on_click(cx.listener(move |this, _, _, cx| {
                        if home && act == StripAct::Home {
                            // Already there: back up to Play.
                            crate::frame::scrolls("home").set_offset(gpui::Point::default());
                        }
                        this.strip_act(&act);
                        cx.notify();
                    })),
                ),
        )
    }
}

/// The strip's own plate: one row, as wide as it is given.
fn strip() -> Div {
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap(px(12.))
        .min_h(px(tokens::ACTION))
        .px(px(14.))
        .py(px(6.))
        .rounded(crate::tokens::r(crate::tokens::R_CARD))
        .bg(palette().card())
        .border_1()
        .border_color(rgb(palette().divider))
        .text_size(px(tokens::LABEL))
        .text_color(rgb(palette().text))
}
