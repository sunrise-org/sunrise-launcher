//! The build window: one line per mod with where it stands, how far the whole build is, and a
//! calm summary when it ends. Closing it never stops the build: the status strip on every tab
//! keeps showing it. What it draws is `build_run::Run`.
use super::build_run::{End, Line, Run, State};
use super::ui::*;
use super::*;
use crate::frame::button;
use crate::paint::palette;
use gpui::{Animation, AnimationExt as _};
use sunrise_launcher::browser::images;
use sunrise_launcher::browser::sunrise;

impl Launcher {
    /// The build window, while one is open.
    pub(crate) fn build_modal(&mut self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let run = self.ui.mods.build.clone()?;
        let running = run.running();
        let counts = run.counts();

        // ---- the head: what this is, for whom, and the way to put it aside
        let close = div()
            .id("build-close")
            .reach()
            .flex_none()
            .size(px(28.))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .hover(|s| s.bg(c(palette().hover)))
            .tooltip(crate::glass_ui::tip(if running {
                tr("Close. The build keeps going in the background.")
            } else {
                tr("Close")
            }))
            .child(blocks::icon("close", 14., c(palette().muted)))
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.close_sheet();
                cx.notify();
            }));
        let head = div()
            .flex()
            .items_start()
            .gap(px(12.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(crate::sheets::title(run.headline()))
                    .child({
                        let build = trf("Sunrise {build}", &[("build", &run.on)]);
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .text_size(px(tokens::LABEL))
                            .text_color(c(palette().muted))
                            .child(
                                div()
                                    .id("build-profile")
                                    .min_w_0()
                                    .truncate()
                                    .tooltip(crate::glass_ui::tip(run.profile.clone()))
                                    .child(run.profile.clone()),
                            )
                            .child(div().flex_none().child("·"))
                            .child(
                                div()
                                    .id("build-context")
                                    .max_w(px(148.))
                                    .flex_none()
                                    .truncate()
                                    .tooltip(crate::glass_ui::tip(build.clone()))
                                    .child(build),
                            )
                    }),
            )
            .child(close);

        // ---- how far the whole build is, or how it went
        let top = if running {
            let percent = (run.fraction() * 100.).round() as u32;
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(
                    div()
                        .flex()
                        .items_baseline()
                        .gap(px(12.))
                        .text_size(px(tokens::LABEL))
                        .child(
                            div()
                                .id("build-step")
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .tooltip(crate::glass_ui::tip(run.step.clone()))
                                .child(run.step.clone()),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_color(c(palette().muted))
                                .child(format!("{percent}%")),
                        ),
                )
                .child(
                    div()
                        .h(px(6.))
                        .rounded(px(3.))
                        .bg(c(palette().line))
                        .overflow_hidden()
                        .child(
                            div()
                                .h(px(6.))
                                .rounded(px(3.))
                                .bg(c(palette().accent))
                                .w(relative(run.fraction().clamp(0.02, 1.))),
                        ),
                )
                .children(
                    run.for_play
                        .then(|| muted(tr("The game starts by itself when this is done."))),
                )
                .children(run.first_build.map(|bytes| {
                    note(trf(
                        "The first build downloads the compiler ({size} MB), once.",
                        &[("size", &bytes.div_ceil(1_000_000).to_string())],
                    ))
                }))
        } else {
            let tally = |n: usize, label: String, icon, color: u32| {
                let live = n > 0;
                let tone = if live { color } else { palette().muted };
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .px(px(10.))
                    .h(px(28.))
                    .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
                    .border_1()
                    .border_color(c(palette().divider))
                    .bg(palette().card())
                    .text_size(px(tokens::LABEL))
                    .when(!live, |d| d.opacity(0.6))
                    .child(crate::glass_ui::icon(icon, 14., c(tone)))
                    .child(label)
            };
            let count = |n: usize| n.to_string();
            div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(px(8.))
                        .child(tally(
                            counts.built,
                            trf("{count} built", &[("count", &count(counts.built))]),
                            "tick-02",
                            palette().success,
                        ))
                        .child(tally(
                            counts.needs_update,
                            match counts.needs_update {
                                1 => tr("1 needs an update").into(),
                                n => trf("{count} need an update", &[("count", &count(n))]),
                            },
                            "information-circle",
                            palette().warning,
                        ))
                        .child(tally(
                            counts.failed,
                            trf("{count} failed", &[("count", &count(counts.failed))]),
                            "cancel-01",
                            palette().danger,
                        )),
                )
                .children(match &run.end {
                    Some(End::Cancelled) => Some(
                        div()
                            .text_size(px(tokens::LABEL))
                            .child(tr("The game does not start. Build again when you are ready.")),
                    ),
                    Some(End::Done) if counts.needs_update + counts.failed > 0 => Some(
                        div()
                            .text_size(px(tokens::LABEL))
                            .child(tr("You can play now. The rest stay installed and come back once they are updated.")),
                    ),
                    _ => None,
                })
        };

        // Completed runs group by outcome; running ones keep the supplied order.
        let mut list = div().flex().flex_col();
        if running {
            for (n, line) in run.lines.iter().enumerate() {
                list = list.child(self.build_line_of(n, line, run.details));
            }
        } else {
            for (group, label, count) in [
                (0, tr("Failed"), counts.failed),
                (1, tr("Needs an update"), counts.needs_update),
                (2, tr("Not built"), counts.other),
                (3, tr("Built"), counts.built),
            ] {
                if count == 0 {
                    continue;
                }
                list = list.child(
                    div()
                        .flex_none()
                        .px(px(14.))
                        .py(px(6.))
                        .text_size(px(tokens::SMALL))
                        .text_color(c(palette().muted))
                        .child(trf(
                            "{label} · {count}",
                            &[("label", label), ("count", &count.to_string())],
                        )),
                );
                for (n, line) in run.lines.iter().enumerate().filter(|(_, l)| {
                    let current = match l.state {
                        State::Failed(_) => 0,
                        State::NeedsUpdate { .. } => 1,
                        State::Built => 3,
                        _ => 2,
                    };
                    current == group
                }) {
                    list = list.child(self.build_line_of(n, line, run.details));
                }
            }
        }
        let stop = match &run.end {
            Some(End::Stopped(said)) => Some(
                muted(if run.details {
                    said.clone()
                } else {
                    said.lines()
                        .find(|line| !line.trim().is_empty())
                        .unwrap_or_default()
                        .to_owned()
                })
                .flex_none()
                .px(px(14.))
                .py(px(10.))
                .when(!run.details, |d| d.line_clamp(2)),
            ),
            _ => None,
        };
        let list = div()
            .id("build-lines")
            .min_h_0()
            .max_h(px(440.))
            .overflow_y_scroll()
            .track_scroll(&crate::frame::scrolls("build-lines"))
            .child(card().flex_none().children(stop).child(list));

        // ---- the foot: stop it, or put it aside
        let has_details =
            run.lines.iter().any(
                |l| matches!(&l.state, State::NeedsUpdate { details, .. } if !details.is_empty()),
            ) || counts.failed > 0
                || matches!(&run.end, Some(End::Stopped(text)) if !text.is_empty());
        let details = (!running && has_details).then(|| {
            div()
                .id("build-details")
                .reach()
                .flex()
                .items_center()
                .gap(px(6.))
                .text_size(px(tokens::LABEL))
                .text_color(c(palette().muted))
                .cursor_pointer()
                .child(blocks::icon(
                    if run.details {
                        "chevron-down"
                    } else {
                        "chevron-right"
                    },
                    12.,
                    c(palette().muted),
                ))
                .child(tr("Details"))
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    if let Some(run) = &mut this.ui.mods.build {
                        run.details = !run.details;
                    }
                    cx.notify();
                }))
        });
        // A build that stopped always offers its log: one that names no mod has no Details.
        let stopped = matches!(run.end, Some(End::Stopped(_)));
        let log = (!running && (run.details || stopped)).then(|| {
            button("build-log", tr("Open the build log")).on_click(cx.listener(
                |this, _: &ClickEvent, _, cx| {
                    let _ = this;
                    crate::glass_toast::open(cx, &native_mods::log_path());
                },
            ))
        });
        let foot = if running {
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .child(
                    button("build-cancel", tr("Cancel build")).on_click(cx.listener(
                        |this, _: &ClickEvent, _, cx| {
                            this.stop_build();
                            cx.notify();
                        },
                    )),
                )
                .child(div().flex_1())
                .child(
                    primary("build-background", tr("Keep building in the background")).on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.close_sheet();
                            cx.notify();
                        }),
                    ),
                )
        } else {
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .children(details)
                .child(div().flex_1())
                .children(log)
                .child(primary("build-done", tr("Done")).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        this.close_sheet();
                        cx.notify();
                    },
                )))
        };

        let body = div()
            .flex()
            .flex_col()
            .child(head.flex_none())
            .child(top.flex_none())
            .child(list)
            .child(foot.flex_none().flex_wrap());
        Some(crate::sheets::bounded_frame("build-window", 600., body, cx))
    }

    /// One mod's line: its picture, its name and version, and where it stands.
    fn build_line_of(&mut self, n: usize, line: &Line, details: bool) -> Div {
        let picture = line
            .icon
            .clone()
            .and_then(|url| self.mods_image(&url, images::ROW));
        // A build that ended never reaches the mods still waiting: they weren't built.
        let ended = self
            .ui
            .mods
            .build
            .as_ref()
            .is_some_and(|run| run.end.is_some());
        let (icon, color, label): (&'static str, u32, String) = match &line.state {
            State::Waiting if ended => ("minus-sign", palette().muted, tr("Not built").into()),
            State::Waiting => ("clock", palette().muted, tr("Waiting").into()),
            State::Building => ("hammer", palette().accent_ink, tr("Building").into()),
            State::Built => ("tick-02", palette().success, tr("Built").into()),
            State::NeedsUpdate { .. } => (
                "information-circle",
                palette().warning,
                tr("Needs an update").into(),
            ),
            State::Skipped(_) => ("minus-sign", palette().muted, tr("Skipped").into()),
            State::Failed(_) => ("cancel-01", palette().danger, tr("Failed").into()),
        };
        // "Needs an update" says it on the right, and the summary above says why for all of
        // them: no sentence repeated under each one.
        let said = match &line.state {
            State::Skipped(why) | State::Failed(why) if details => Some(why.clone()),
            _ => None,
        };
        let technical = match &line.state {
            State::NeedsUpdate { details: lines, .. } if details => lines.clone(),
            _ => Vec::new(),
        };
        let glyph = crate::glass_ui::icon(icon, 14., c(color));
        let building = line.state == State::Building;
        let mark = if building && !crate::motion::still() {
            glyph
                .with_animation(
                    ElementId::named_usize("build-line-now", n),
                    Animation::new(Duration::from_millis(1200))
                        .repeat()
                        .with_easing(gpui::ease_in_out),
                    |g, t| g.opacity(0.45 + 0.55 * (1. - (2. * t - 1.).abs())),
                )
                .into_any_element()
        } else {
            glyph.into_any_element()
        };
        let waiting = line.state == State::Waiting;
        div()
            .flex()
            .flex_col()
            .flex_none()
            .when(n > 0, |d| d.border_t_1().border_color(c(palette().divider)))
            .when(building, |d| d.bg(c(palette().selected)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .px(px(14.))
                    .min_h(px(52.))
                    .child(
                        tile(picture, &line.id, &line.title, 32.).when(waiting, |d| d.opacity(0.6)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .py(px(8.))
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(
                                div()
                                    .flex()
                                    .items_baseline()
                                    .gap(px(8.))
                                    .child(
                                        div()
                                            .id(("build-mod-title", n))
                                            .min_w_0()
                                            .tooltip(crate::glass_ui::tip(format!(
                                                "{} · {}",
                                                line.title, line.id
                                            )))
                                            .truncate()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .when(waiting, |d| d.text_color(c(palette().muted)))
                                            .child(line.title.clone()),
                                    )
                                    .child(
                                        div()
                                            .id(("build-mod-version", n))
                                            .flex_none()
                                            .max_w(px(96.))
                                            .truncate()
                                            .tooltip(crate::glass_ui::tip(line.version.clone()))
                                            .text_size(px(tokens::SMALL))
                                            .font_family(crate::theme::MONO)
                                            .text_color(c(palette().muted))
                                            .child(line.version.clone()),
                                    ),
                            )
                            .children(said.map(muted)),
                    )
                    .child(
                        div()
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .text_size(px(tokens::LABEL))
                            .text_color(c(color))
                            .child(mark)
                            .child(label),
                    ),
            )
            .children((!technical.is_empty()).then(|| {
                div()
                    .mx(px(14.))
                    .mb(px(12.))
                    .p(px(10.))
                    .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
                    .bg(c(palette().field))
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .text_size(px(tokens::SMALL))
                    .font_family(crate::theme::MONO)
                    .text_color(c(palette().muted))
                    .children(technical.into_iter().map(crate::selectable::selectable))
            }))
    }
}

impl Launcher {
    /// The one Build of Mods › Installed: every mod that waits, in one build. While a build
    /// runs it says how far that is, over a thin fill, and opens the build window again.
    pub(super) fn build_button(
        &mut self,
        waiting: usize,
        busy: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let running = self.build_here().filter(|run| run.running());
        let label = match (running.map(Run::at), waiting) {
            (Some(Some((at, total))), _) => trf(
                "Building · {at} of {total}",
                &[("at", &at.to_string()), ("total", &total.to_string())],
            ),
            (Some(None), _) => tr("Building…").into(),
            (None, 1) => tr("Build 1 mod").into(),
            (None, n) => trf("Build {count} mods", &[("count", &n.to_string())]),
        };
        let fill = running.map(|run| {
            div()
                .absolute()
                .left_0()
                .right_0()
                .bottom_0()
                .h(px(3.))
                .bg(c(palette().line))
                .child(
                    div()
                        .h(px(3.))
                        .bg(c(palette().accent))
                        .w(relative(run.fraction().clamp(0.02, 1.))),
                )
        });
        let b = match fill {
            Some(fill) => crate::frame::button("alt-build-all", label)
                .relative()
                .overflow_hidden()
                .child(fill),
            None => primary("alt-build-all", label),
        };
        // Another change holds the folder, or another profile's build runs: one at a time.
        if running.is_none() && (busy || self.native_step.is_some()) {
            return dim(b);
        }
        b.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
            this.build_all();
            cx.notify();
        }))
    }

    /// Build: starts the selected profile's build and opens its window. While a build runs it
    /// only opens the window again, wherever it is pressed (Mods, the strip, Home).
    pub(crate) fn build_all(&mut self) {
        if self.native_step.is_none() {
            // A build that first asks for permission opens its window once it starts.
            self.ui.mods.build_wanted = true;
            self.build();
        }
        if self.ui.mods.build.as_ref().is_some_and(Run::running) {
            self.ui.mods.build_wanted = false;
            self.open_sheet(crate::sheets::Sheet::ModBuild);
        }
    }

    /// Cancel build in the window, Stop on the strip and on Home: the build is told to stop,
    /// and from this press on it starts no game, even if it ends well before it hears.
    pub(crate) fn stop_build(&mut self) {
        build_run::stop_asked(self.ui.mods.build.as_mut(), &mut self.play_after_build);
        if let Some(cancellation) = &self.native_cancellation {
            cancellation.stop();
        }
    }

    /// Asks the registry again whether a newer version of mod `id` is out, and says what it
    /// found. A newer version then shows on the mod's row, with Update.
    pub(crate) fn check_mod_update(&mut self, id: &str) {
        let Some(row) = self.rows().into_iter().find(|r| r.id == id) else {
            return;
        };
        let (tx, game) = (self.ui.mods.tx.clone(), self.mods_game());
        let project = row.entry.as_ref().map(|e| e.project.clone());
        std::thread::spawn(move || {
            let found = sunrise::refreshed_index().map(|index| {
                project
                    .as_deref()
                    .and_then(|id| index.project(id))
                    .and_then(|p| model::newer(&index, p, &row.version, &game))
            });
            let _ = tx.send(Event::Checked(row.title, found));
        });
    }

    /// What [`Self::check_mod_update`] found, said once.
    pub(super) fn checked(&mut self, title: String, found: Result<Option<String>, String>) {
        crate::glass_toast::say(match found {
            Ok(Some(version)) => trf(
                "{mod} {version} is out. Update it under Mods.",
                &[("mod", &title), ("version", &version)],
            ),
            Ok(None) => trf("No newer version of {mod} yet.", &[("mod", &title)]),
            Err(error) => error,
        });
    }
}
