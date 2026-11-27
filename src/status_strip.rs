//! The status strip: what Play is doing or what went wrong, with its one action, on every page
//! except Home, whose fixed Play bar always shows this feedback. Source builds are said here
//! on every page, Home included.
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
        use crate::mods_page::install::{stage_fraction, stage_label};
        use sunrise_launcher::browser::sunrise::InstallProgress;
        if let Some(activity) = self.source_activity() {
            let step = activity.progress;
            let label = match step {
                InstallProgress::Publishing => tr("Finishing…").into(),
                _ if activity.stopping => tr("Stopping…").into(),
                InstallProgress::Tools { done, total } if total > 0 => trf(
                    "Getting build tools · {done} of {total}",
                    &[
                        ("done", &sunrise_launcher::storage::format_bytes(done)),
                        ("total", &sunrise_launcher::storage::format_bytes(total)),
                    ],
                ),
                InstallProgress::Building(_) => trf(
                    "Building {mods}…",
                    &[("mods", &crate::mods::names(activity.names))],
                ),
                _ => stage_label(step),
            };
            let label = if step == InstallProgress::Publishing || activity.stopping {
                label
            } else {
                crate::eta::with_eta(label, self.registry_eta())
            };
            let stop = (step != InstallProgress::Publishing && !activity.stopping).then(|| {
                button("nightly-source-stop", tr("Stop")).on_click(cx.listener(|this, _, _, cx| {
                    this.stop_source();
                    cx.notify();
                }))
            });
            let fraction = stage_fraction(step).map(|f| (f, f));
            return Some(
                strip()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .child(div().truncate().child(label))
                            .child(crate::widgets::progress_bar(fraction, &self.theme)),
                    )
                    .children(stop),
            );
        }
        if self.ui.page == crate::frame::Page::Home {
            return None;
        }
        self.refresh_home_inventory(cx);
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
            StripAct::StopInstall => "alt-strip-stop-install",
            StripAct::Act(_) => "alt-strip-fix",
            StripAct::Home => "alt-strip-home",
        };
        let stopping_install = act == StripAct::StopInstall && self.registry_install_stopping();
        let line = if stopping_install {
            div().truncate().child(tr("Stopping…")).into_any_element()
        } else {
            match self.build_line(cx).filter(|_| view.main == Main::Building) {
                Some(build) => build.into_any_element(),
                None => div().truncate().child(says).into_any_element(),
            }
        };
        let mark = if view.failed {
            palette().warning
        } else {
            palette().accent
        };
        // A download says how far it is in the label; its bar shows it too.
        let fraction = match view.main {
            Main::Installing(phase) => stage_fraction(phase.into()),
            Main::RegistryInstall(stage, _) => stage_fraction(stage),
            _ => None,
        };
        let bar = fraction.map(|progress| {
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
                )
        });
        Some(
            strip()
                .child(div().flex_none().size(px(8.)).rounded_full().bg(rgb(mark)))
                .child(div().flex_1().min_w_0().child(line))
                .children(bar)
                .when(!stopping_install, |d| {
                    d.child(
                        button(id, act.label()).on_click(cx.listener(move |this, _, _, cx| {
                            this.strip_act(&act);
                            cx.notify();
                        })),
                    )
                }),
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
