//! The build window: one line per mod with where it stands, how far the whole build is, and a
//! calm summary when it ends. Closing it never stops the build: the status strip on every tab
//! keeps showing it. What it draws is `build_run::Run`.
use super::build_run::{End, Line, Recovery, Run, State};
use super::ui::*;
use super::*;
use crate::paint::palette;
use crate::widgets::button::{primary, secondary};
use gpui::{Animation, AnimationExt as _};
use sunrise_launcher::browser::images;
use sunrise_launcher::browser::sunrise;

/// How many of the compiler's latest lines the live output shows (the pane holds about this many).
const LIVE_LINES: usize = 12;
/// How often the live output is read again while it shows.
const LIVE_REFRESH: std::time::Duration = std::time::Duration::from_millis(500);

impl Launcher {
    /// The build window, while one is open.
    pub(crate) fn build_modal(&mut self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let run = self.mods_state.build.clone()?;
        let running = run.running();
        let counts = run.counts();
        let stop = match &run.end {
            Some(End::Stopped(stop)) => Some(stop),
            _ => None,
        };

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
                    .child(muted(trf(
                        "{profile} · Sunrise {build}",
                        &[("profile", &run.profile), ("build", &run.on)],
                    ))),
            )
            .child(close);

        // ---- how far the whole build is, or how it went
        let top = if running {
            let fraction = run.fraction();
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
                        .child(div().flex_1().min_w_0().truncate().child(run.step.clone()))
                        .child(
                            div()
                                .flex_none()
                                .text_color(c(palette().muted))
                                .child(format!("{}%", (fraction * 100.).round() as u32)),
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
                                .w(relative(fraction.clamp(0.02, 1.))),
                        ),
                )
                .children(
                    (run.for_play && !run.stop_requested)
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
                    Some(End::Stopped(stop)) => Some(muted(stop.summary.clone())),
                    Some(End::Cancelled) => Some(
                        div().text_size(px(tokens::LABEL)).child(tr("The game does not start. Build again when you are ready.")),
                    ),
                    Some(End::Done) if counts.needs_update + counts.failed > 0 => Some(
                        div()
                            .text_size(px(tokens::LABEL))
                            .child(tr("You can play now. The rest stay installed and come back once they are updated.")),
                    ),
                    _ => None,
                })
        };

        // ---- one line per mod
        let mut list = div().flex().flex_col();
        for (n, line) in run.lines.iter().enumerate() {
            list = list.child(self.build_line_of(n, line, run.details, !running));
        }
        let error = stop
            .map(|stop| stop.error.as_str())
            .filter(|s| !s.is_empty());
        // While it runs, the compiler's latest lines as the output reader last found them.
        let live = (running && run.output_open).then(|| {
            run.output
                .clone()
                .unwrap_or_else(|| tr("Waiting for the compiler's first lines…").to_owned())
        });
        let diagnostic = if live.is_some() {
            live.as_deref()
        } else if run.log_open {
            run.log.as_deref()
        } else {
            error.filter(|_| run.details)
        };
        let log_height = if diagnostic.is_some() { 165. } else { 0. };
        let list = card().overflow_hidden().child(
            div()
                .id("build-lines")
                .max_h(px((self.window_height - 330. - log_height).max(96.)))
                .overflow_y_scroll()
                .track_scroll(&crate::frame::scrolls("build-lines"))
                .child(list),
        );

        // ---- the foot: stop it, or put it aside
        let has_details = error.is_some()
            || run.lines.iter().any(
                |l| matches!(&l.state, State::NeedsUpdate { details, .. } if !details.is_empty()),
            );
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
                .child(crate::widgets::motion_bits::disclosure(
                    "build-details",
                    run.details,
                    12.,
                    c(palette().muted),
                ))
                .child(if run.details {
                    tr("Hide details")
                } else {
                    tr("Show details")
                })
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    if let Some(run) = &mut this.mods_state.build {
                        run.details = !run.details;
                        run.log_open = false;
                    }
                    cx.notify();
                }))
        });
        // While it runs: show or hide the compiler's output as it comes.
        let output = running.then(|| Self::build_output_button(run.output_open, cx));
        // Managed Play only offers the log of this attempt, never the latest on disk.
        let log = (!running
            && (run.details || stop.is_some())
            && (!run.for_play || run.log.is_some()))
        .then(|| {
            let label = if run.log_open {
                tr("Hide build log")
            } else {
                tr("Open the build log")
            };
            secondary("build-log", label).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                if let Some(run) = &mut this.mods_state.build {
                    if run.for_play {
                        run.log_open = !run.log_open;
                        run.details = false;
                        cx.notify();
                    } else {
                        crate::toast::open(cx, &native_mods::log_path());
                    }
                }
            }))
        });
        let request = self.play_retry.as_ref().filter(|request| {
            request.instance == run.instance
                && self
                    .selected_instance()
                    .is_some_and(|instance| request.matches_profile(instance))
        });
        let (recovery, refusal) = match (stop.and_then(|stop| stop.action.clone()), request) {
            (Some(Recovery::Retry), Some(_)) => (Some((tr("Try again").to_owned(), None)), None),
            (Some(Recovery::TurnOff { label, mods }), Some(request)) => {
                match request.destination.check_mod_recovery() {
                    Err(reason) => (None, Some(reason)),
                    Ok(()) => (
                        run.failed_build.clone().map(|build| {
                            (
                                label,
                                Some(sunrise_launcher::managed_play::ModRecovery { build, mods }),
                            )
                        }),
                        None,
                    ),
                }
            }
            _ => (None, None),
        };
        let copy = diagnostic.map(|diagnostic| {
            let diagnostic = diagnostic.to_owned();
            let copy_id: SharedString = if run.log_open {
                "copy-build-log"
            } else {
                "copy-build-error"
            }
            .into();
            secondary(
                copy_id.clone(),
                if crate::widgets::copies::copied(&copy_id).is_some() {
                    tr("Copied")
                } else if run.log_open {
                    tr("Copy")
                } else {
                    tr("Copy error")
                },
            )
            .on_click(move |_, window, cx| {
                crate::selectable::clipboard::copy_quietly(
                    cx,
                    gpui::ClipboardItem::new_string(diagnostic.clone()),
                );
                crate::widgets::copies::note_copy(copy_id.clone(), window, cx);
            })
        });
        let foot = if running {
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .children(output)
                .child(
                    secondary("build-cancel", tr("Cancel build")).on_click(cx.listener(
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
            let finish = div()
                .ml_auto()
                .flex()
                .flex_wrap()
                .justify_end()
                .gap(px(10.))
                .children(recovery.map(|(label, recovery)| {
                    secondary("play-build-retry", label).on_click(cx.listener(
                        move |this, _, _, cx| {
                            this.retry_managed_play(recovery.clone());
                            cx.notify();
                        },
                    ))
                }))
                .child(primary("build-done", tr("Done")).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        this.close_sheet();
                        cx.notify();
                    },
                )));
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(10.))
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap(px(10.))
                        .children(details)
                        .children(copy)
                        .children(log),
                )
                .child(finish)
        };

        let body = div()
            .flex()
            .flex_col()
            .child(head)
            .child(top)
            .children(refusal.map(muted))
            .child(list)
            .children(diagnostic.map(|text| {
                card().min_w_0().overflow_hidden().child(
                    div()
                        .id(if live.is_some() {
                            "build-output-pane"
                        } else if run.log_open {
                            "build-log-pane"
                        } else {
                            "build-error-pane"
                        })
                        .max_h(px(150.))
                        .min_w_0()
                        .overflow_y_scroll()
                        .on_scroll(|_, _, cx| cx.stop_propagation())
                        .p(px(12.))
                        .font_family(crate::theme::MONO)
                        .text_size(px(tokens::SMALL))
                        .whitespace_normal()
                        .child(
                            crate::selectable::Selectable::styled(
                                "build-diagnostic-text",
                                gpui::StyledText::new(text.to_owned()),
                            )
                            .without_copy_preview(),
                        ),
                )
            }))
            .child(foot);
        Some(crate::sheets::frame("build-window", 600., body, cx))
    }

    /// Show output / Hide output, while a build runs.
    fn build_output_button(open: bool, cx: &mut Context<Self>) -> Stateful<Div> {
        let label = if open {
            tr("Hide output")
        } else {
            tr("Show output")
        };
        secondary("build-output", label).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
            let Some(run) = &mut this.mods_state.build else {
                return;
            };
            run.output_open = !run.output_open;
            if run.output_open {
                Self::read_build_output(run, cx);
            }
            cx.notify();
        }))
    }

    /// Reads the compiler's latest lines off the UI thread, now and every [`LIVE_REFRESH`],
    /// while `run` shows them. At most one reader per build: reopening reuses a running one.
    fn read_build_output(run: &mut Run, cx: &mut Context<Self>) {
        if run.output_reader.is_some() {
            return;
        }
        let token = std::sync::Arc::new(());
        run.output_reader = Some(token.clone());
        cx.spawn(async move |this, cx| loop {
            let text = cx
                .background_executor()
                .spawn(async { native_mods::live_tail(LIVE_LINES) })
                .await;
            let Ok(more) = this.update(cx, |this, cx| {
                let Some(run) = this.mods_state.build.as_mut() else {
                    return false;
                };
                // Another build's run, or a newer reader for this one: leave it alone.
                if !run
                    .output_reader
                    .as_ref()
                    .is_some_and(|reader| std::sync::Arc::ptr_eq(reader, &token))
                {
                    return false;
                }
                if !(run.output_open && run.running()) {
                    run.output_reader = None;
                    return false;
                }
                if run.output != text {
                    run.output = text;
                    cx.notify();
                }
                true
            }) else {
                break;
            };
            if !more {
                break;
            }
            cx.background_executor().timer(LIVE_REFRESH).await;
        })
        .detach();
    }

    /// One mod's line: its picture, its name and version, and where it stands.
    fn build_line_of(&mut self, n: usize, line: &Line, details: bool, ended: bool) -> Div {
        let picture = line
            .icon
            .clone()
            .and_then(|url| self.mods_image(&url, images::ROW));
        // A build that ended never reaches the mods still waiting: they weren't built.
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
            State::Skipped(why) | State::Failed(why) => Some(why.clone()),
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
                    crate::motion::endless(
                        Animation::new(Duration::from_millis(1200))
                            .repeat()
                            .with_easing(gpui::ease_in_out),
                    ),
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
                                            .min_w_0()
                                            .when(!matches!(line.state, State::Failed(_)), |d| {
                                                d.truncate()
                                            })
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .when(waiting, |d| d.text_color(c(palette().muted)))
                                            .child(line.title.clone()),
                                    )
                                    .child(
                                        div()
                                            .flex_none()
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
                crate::code_box::code_box(
                    SharedString::from(format!("build-line-technical-{n}")),
                    technical.into_iter().map(Into::into).collect(),
                    None,
                )
                .mx(px(14.))
                .mb(px(12.))
                .border_0()
            }))
    }
}

impl Launcher {
    /// Build: starts the selected profile's build and opens its window. While a build runs it
    /// only opens the window again, wherever it is pressed (Mods, the strip, Home).
    pub(crate) fn build_all(&mut self) {
        if self.native_step.is_none() {
            // A build that first asks for permission opens its window once it starts.
            self.mods_state.build_wanted = true;
            self.build();
        }
        if self.mods_state.build.as_ref().is_some_and(Run::running) {
            self.mods_state.build_wanted = false;
            self.open_sheet(crate::sheets::Sheet::ModBuild);
        }
    }

    /// Cancel build in the window, Stop on the strip and on Home: the build is told to stop,
    /// and from this press on it starts no game, even if it ends well before it hears.
    pub(crate) fn stop_build(&mut self) {
        build_run::stop_asked(self.mods_state.build.as_mut(), &mut self.play_after_build);
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
        let base = self
            .mods_state
            .snapshot
            .as_ref()
            .and_then(|snap| snap.rows.managed.as_ref())
            .map(|managed| managed.base().to_owned())
            .or_else(|| self.selected_exact_build());
        let tx = self.mods_state.tx.clone();
        let entry = row.entry.clone();
        std::thread::spawn(move || {
            let found = sunrise::v2::refreshed_index()
                .map_err(|error| error.to_string())
                .map(|catalog| {
                    let base = base.as_deref()?;
                    let unfit_updates = sunrise::known_unfit_updates(base);
                    let entry = entry.as_ref()?;
                    let project = sunrise::v2::installed_project(&catalog.index, entry)?;
                    sunrise::v2::newer_release(
                        &catalog.index,
                        project,
                        &entry.version,
                        base,
                        &unfit_updates,
                    )
                    .map(|release| release.version.clone())
                });
            let _ = tx.send(Event::Checked(row.title, found));
        });
    }

    /// What [`Self::check_mod_update`] found, said once.
    pub(super) fn checked(&mut self, title: String, found: Result<Option<String>, String>) {
        crate::toast::say(match found {
            Ok(Some(version)) => trf(
                "{mod} {version} is out. Update it under Mods.",
                &[("mod", &title), ("version", &version)],
            ),
            Ok(None) => trf("No newer version of {mod} yet.", &[("mod", &title)]),
            Err(error) => error,
        });
    }
}
