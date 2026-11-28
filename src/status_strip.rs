//! The status strip, above the page on every tab: the work under way (`jobs`), each job in the
//! same row (`list`), and, off Home, what went wrong with Play or waits on it, with its one
//! action; Home's Play bar says that part itself.
//! It is not drawn at all while there is nothing to tell (`launch_state::View::strip`).
use super::*;
use crate::launch_state::{Act, Main, StripAct};
use crate::paint::palette;
use crate::widgets::button::secondary;
use gpui::rgb;

mod jobs;
mod list;
pub(crate) use jobs::Jobs;
#[cfg(test)]
mod tests;

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
                    d.child(secondary("play-registry-details", tr("Details…")).on_click(
                        cx.listener(|this, _, _, cx| {
                            this.open_notice(this.play_registry_notices.join("\n"), None);
                            cx.notify();
                        }),
                    ))
                })
                .child(
                    secondary("play-registry-dismiss", tr("Dismiss")).on_click(cx.listener(
                        |this, _, _, cx| {
                            this.play_registry_notices.clear();
                            cx.notify();
                        },
                    )),
                ),
        )
    }

    /// The strip, for the frame to place above the page, as wide as it is given: the work under
    /// way on every page, Home included, then (off Home) what failed or waits on Play.
    pub(crate) fn status_strip(&mut self, cx: &mut Context<Self>) -> Option<Div> {
        let jobs = self.jobs();
        let said = self.said_strip(!jobs.is_empty(), cx);
        let work = (!jobs.is_empty()).then(|| self.jobs_strip(&jobs, cx));
        if work.is_none() && said.is_none() {
            return None;
        }
        Some(
            div()
                .flex_none()
                .flex()
                .flex_col()
                .gap(px(6.))
                .children(work)
                .children(said),
        )
    }

    /// Play's own line off Home: what failed, a held update or the game running, with its one
    /// action. Work under way is the job list's (`covered`); this says it only for work no job
    /// describes.
    fn said_strip(&mut self, covered: bool, cx: &mut Context<Self>) -> Option<Div> {
        if self.ui.page == crate::frame::Page::Home {
            return None;
        }
        self.refresh_home_inventory(cx);
        let view = self.launch_facts().view();
        if covered && view.main.under_way() {
            return None;
        }
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
            StripAct::Stop(_) => "alt-strip-stop-work",
            StripAct::Act(_) => "alt-strip-fix",
            StripAct::Home => "alt-strip-home",
        };
        let stopping_install = (act == StripAct::StopInstall && self.registry_install_stopping())
            || (matches!(act, StripAct::Stop(_)) && view.stopping);
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
            Main::Installing(progress) => Some(progress),
            Main::RegistryInstall(stage, _) => crate::mods_page::install::stage_fraction(stage),
            _ => None,
        };
        let bar = fraction.map(|progress| {
            div()
                .flex_none()
                .w(px(120.))
                .child(crate::widgets::feedback::progress_bar(Some(progress)))
        });
        Some(
            strip()
                .child(div().flex_none().size(px(8.)).rounded_full().bg(rgb(mark)))
                .child(div().flex_1().min_w_0().child(line))
                .children(bar)
                .when(!stopping_install, |d| {
                    d.child(secondary(id, act.label()).on_click(cx.listener(
                        move |this, _, _, cx| {
                            this.strip_act(&act);
                            cx.notify();
                        },
                    )))
                }),
        )
    }
}

/// The strip's own plate: one row, as wide as it is given.
fn strip() -> Div {
    plate().flex().items_center().gap(px(12.))
}

/// The plate every strip is drawn on.
fn plate() -> Div {
    div()
        .flex_none()
        .min_h(px(tokens::ACTION))
        .px(px(14.))
        .py(px(6.))
        .rounded(crate::tokens::r(crate::tokens::R_CARD))
        .bg(palette().card())
        .border_1()
        .border_color(palette().rule())
        .text_size(px(tokens::LABEL))
        .text_color(rgb(palette().text))
}
