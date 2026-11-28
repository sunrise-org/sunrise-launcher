//! "A specific version…": the sheet that picks one build to keep, however many there are.
//!
//! One list, drawn only where it is on screen: what is installed first (the build kept or played
//! leading), then every build that can be downloaded, by channel and month, newest first. The
//! field narrows it by date, name or commit. A row only chooses; confirming starts the
//! staged switch. Download and Prepare do not change the playing build.
mod build_list;
#[cfg(test)]
mod tests;
mod wheel_bench;

use super::*;
use crate::line_edit::{self, LineEdit};
use crate::paint::palette;
use std::{
    path::PathBuf,
    rc::Rc,
    sync::{Arc, OnceLock},
};
use sunrise_launcher::{backend::Release, browser::sunrise::BuildChoice, instances::Channel};

use build_list::*;
pub(crate) use wheel_bench::*;

/// The height of every line of the list, headings included.
const LINE_H: f32 = 40.;

struct FitPreview {
    home: PathBuf,
    tag: String,
    checked: Arc<OnceLock<bool>>,
}

#[derive(Default)]
pub(crate) struct State {
    find: LineEdit,
    focus: Option<FocusHandle>,
    /// Whether the field has the keyboard, read where the window is at hand.
    focused: bool,
    /// The build chosen in the list, before it is kept.
    pick: Option<String>,
    /// The same-channel update that opened the list, if any. It does not pin the chosen build.
    update: Option<String>,
    preview: Option<FitPreview>,
    /// The list's scroll, which the arrow keys move to the row they land on.
    scroll: gpui::UniformListScrollHandle,
    /// The keyboard focus of each row, by line: a row that is not drawn has no element, so the
    /// handles live here for the arrow keys to land on a row before it is on screen.
    rows: std::collections::HashMap<usize, FocusHandle>,
}

impl Launcher {
    /// Opens the sheet on the build the setup keeps now.
    pub(crate) fn open_versions(&mut self) {
        self.open_sheet(Sheet::Versions);
        self.reset_versions();
    }

    pub(crate) fn reset_versions(&mut self) {
        self.ui.sheets.error = None;
        let pinned = self
            .selected_instance()
            .and_then(|i| self.profile_target(i));
        let v = &mut self.ui.sheets.versions;
        v.find.set(String::new());
        v.pick = pinned;
        v.update = None;
        v.preview = None;
        v.rows.clear();
    }

    /// Reads whether the field has the keyboard, before the sheet is drawn.
    pub(crate) fn versions_focus(&mut self, window: &Window) {
        let v = &mut self.ui.sheets.versions;
        v.focused = v.focus.as_ref().is_some_and(|f| f.is_focused(window));
    }

    pub(super) fn versions_sheet(&mut self, cx: &mut Context<Self>) -> Stateful<Div> {
        let body = self.versions_content(cx);
        frame("alt-versions", 560., body, cx)
    }

    pub(crate) fn open_update_picker(&mut self, tag: String) {
        self.open_versions();
        self.ui.sheets.versions.update = Some(tag.clone());
        self.ui.sheets.versions.pick = Some(tag);
    }

    fn architecture_repair(&self, tag: &str) -> Option<&Release> {
        let Load::Ready(releases) = &self.releases else {
            return None;
        };
        backend::architecture_reinstall(releases, tag, self.settings.game_architecture)
    }

    pub(crate) fn can_repair_now(&self, tag: &str) -> bool {
        !self.launch_busy() && self.client.is_none() && !self.removing.iter().any(|t| t == tag)
    }

    /// The list the picker shows for `instance`, whose Play target is `plays`.
    fn version_lines(&self, instance: &instances::Instance, plays: Option<&str>) -> Vec<Line> {
        let releases: &[Release] = match &self.releases {
            Load::Ready(rows) => rows,
            _ => &[],
        };
        lines(
            releases,
            &self.installed,
            plays,
            instance.channel,
            self.ui.sheets.versions.find.text(),
        )
    }

    /// How tall the list of `lines` is drawn: as tall as what it lists, up to the room the
    /// window has; past that it scrolls.
    fn versions_list_height(&self, lines: usize) -> f32 {
        (lines as f32 * LINE_H).min((self.window_height - 330.).clamp(LINE_H * 4., LINE_H * 9.))
    }

    fn versions_content(&mut self, cx: &mut Context<Self>) -> Div {
        let Some(instance) = self.selected_instance() else {
            return div().child(tr("No profile selected."));
        };
        let plays = self.profile_target(instance);
        let is_pack = instance.kind == instances::Kind::Pack;
        let home = instance.home.clone();
        let current = plays.as_deref();
        let query = self.ui.sheets.versions.find.text().to_owned();
        let list = Rc::new(self.version_lines(instance, current));
        let list_height = self.versions_list_height(list.len());
        let pick = self.ui.sheets.versions.pick.clone();
        let is_update = pick.is_some() && pick == self.ui.sheets.versions.update;
        let choice = (!is_update || instance.version.is_some()).then_some(BuildChoice::Keep);
        let already_kept = instance.version.as_deref() == current;

        let focus = self
            .ui
            .sheets
            .versions
            .focus
            .get_or_insert_with(|| cx.focus_handle().tab_index(0).tab_stop(true))
            .clone();
        let focused = self.ui.sheets.versions.focused;
        let field = crate::frame::input(focused)
            .id("alt-versions-find")
            .track_focus(&focus)
            .seen()
            .h(px(tokens::FIELD))
            .px(px(12.))
            .text_size(px(tokens::BODY))
            .cursor_text()
            .child(line_edit::content(
                &self.ui.sheets.versions.find,
                &self.theme,
                focused,
                false,
                false,
                "Search by date or commit…",
            ))
            .on_click(cx.listener(move |_, _: &ClickEvent, window, cx| {
                window.focus(&focus, cx);
                cx.notify();
            }))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                match line_edit::handle(&mut this.ui.sheets.versions.find, ev, cx) {
                    // Escape leaves the field first; the sheet closes on the next one.
                    line_edit::Key::Escape => window.blur(cx),
                    line_edit::Key::Pass => return,
                    _ => {}
                }
                cx.stop_propagation();
                cx.notify();
            }));
        let field = line_edit::mouse(field, cx, |this| &mut this.ui.sheets.versions.find);

        let shown = Rc::clone(&list);
        let (now, chosen) = (plays.clone(), pick.clone());
        let scroll = self.ui.sheets.versions.scroll.clone();
        let rows = gpui::uniform_list(
            "alt-versions-list",
            list.len(),
            cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                range
                    .map(|n| match &shown[n] {
                        Line::Head(text) => div()
                            .id(("alt-version", n))
                            .h(px(LINE_H))
                            .px(px(14.))
                            .flex()
                            .items_end()
                            .pb(px(6.))
                            .text_size(px(tokens::CAPTION))
                            .text_color(rgb(palette().muted))
                            .child(text.to_uppercase()),
                        Line::Build(b) => {
                            let on = chosen.as_deref() == Some(b.tag.as_str());
                            let playing = now.as_deref() == Some(b.tag.as_str());
                            let tag = b.tag.clone();
                            let handle = this
                                .ui
                                .sheets
                                .versions
                                .rows
                                .entry(n)
                                .or_insert_with(|| cx.focus_handle().tab_index(0).tab_stop(true))
                                .clone();
                            div()
                                .id(("alt-version", n))
                                .w_full()
                                .h(px(LINE_H))
                                .px(px(14.))
                                .flex()
                                .items_center()
                                .gap(px(12.))
                                .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
                                .when(on, |d| d.bg(rgb(palette().selected)))
                                .child(
                                    div()
                                        .size(px(14.))
                                        .flex_none()
                                        .rounded_full()
                                        .border_1()
                                        .border_color(rgb(if on {
                                            palette().text
                                        } else {
                                            palette().muted
                                        }))
                                        .when(on, |d| d.bg(rgb(palette().text))),
                                )
                                .child(
                                    div()
                                        .flex_none()
                                        .text_size(px(tokens::BODY))
                                        .child(b.name.clone()),
                                )
                                .child(
                                    muted(b.about.clone())
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_size(px(tokens::SMALL)),
                                )
                                .children(playing.then(|| {
                                    muted(tr("Playing now"))
                                        .flex_none()
                                        .text_size(px(tokens::SMALL))
                                }))
                                .children((b.installed && !playing).then(|| {
                                    muted(tr("Installed"))
                                        .flex_none()
                                        .text_size(px(tokens::SMALL))
                                }))
                                .track_focus(&handle)
                                .reach()
                                .cursor_pointer()
                                .hover(|s| s.bg(rgb(palette().hover)))
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    let v = &mut this.ui.sheets.versions;
                                    if v.pick.as_ref() != Some(&tag) {
                                        v.preview = None;
                                        v.pick = Some(tag.clone());
                                    }
                                    cx.notify();
                                }))
                        }
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(&scroll)
        .h(px(list_height));
        let nothing = list.is_empty().then(|| {
            muted(match (&self.releases, query.trim().is_empty()) {
                (Load::Loading, _) => tr("Reading the list of builds…"),
                (_, false) => tr("No build matches your search."),
                (_, true) => tr("No builds to choose from yet."),
            })
            .px(px(14.))
            .py(px(12.))
        });

        let cancel = button("alt-versions-cancel", tr("Cancel")).on_click(cx.listener(
            |this, _: &ClickEvent, _, cx| {
                this.close_sheet();
                cx.notify();
            },
        ));
        let reinstall = pick.as_ref().and_then(|tag| {
            self.architecture_repair(tag)?;
            let tag = tag.clone();
            let label = match &self.installing {
                Some((started, progress)) if started == &tag => trf(
                    "Installing · {percent}%",
                    &[("percent", &format!("{:.0}", progress * 100.))],
                ),
                _ => tr("Reinstall for this architecture").to_owned(),
            };
            let button = button("alt-versions-reinstall", label);
            Some(if self.can_repair_now(&tag) {
                button.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    // Recheck the exact offer and the active jobs at click.
                    if this.can_repair_now(&tag) {
                        if let Some(release) = this.architecture_repair(&tag).cloned() {
                            this.architecture_reinstall = Some(tag.clone());
                            this.install(&release);
                        }
                    }
                    cx.notify();
                }))
            } else {
                button.opacity(0.45).cursor_default().tab_stop(false)
            })
        });
        let idle = !self.launch_busy() && self.client.is_none();
        let mut cache_action = None;
        let mut plan = None;
        let mut keep_tag = None;
        if let Some(tag) = pick.as_ref() {
            match version_action(
                self.installed.contains(tag),
                || backend::is_prepared(tag),
                !(is_pack || already_kept && Some(tag.as_str()) == current),
            ) {
                Some(VersionAction::Choose) => {
                    if idle {
                        keep_tag = Some(tag.clone());
                    }
                    if Some(tag.as_str()) != current {
                        let request = &mut self.ui.sheets.versions.preview;
                        if request
                            .as_ref()
                            .is_none_or(|preview| preview.home != home || preview.tag != *tag)
                        {
                            let checked = Arc::new(OnceLock::new());
                            *request = Some(FitPreview {
                                home: home.clone(),
                                tag: tag.clone(),
                                checked: checked.clone(),
                            });
                            let target = tag.clone();
                            let work = cx.background_executor().spawn(async move {
                                sunrise_launcher::browser::sunrise::v2::index().is_ok_and(
                                    |catalog| {
                                        sunrise_launcher::browser::sunrise::checked_switch_fit_v2(
                                            &catalog.index,
                                            &home,
                                            &target,
                                        )
                                    },
                                )
                            });
                            cx.spawn(async move |this, cx| {
                                let _ = checked.set(work.await);
                                let _ = this.update(cx, |_, cx| cx.notify());
                            })
                            .detach();
                        }
                        plan = Some(
                            if request.as_ref().and_then(|p| p.checked.get()) == Some(&true) {
                                tr("Every mod in this profile has a release for this build.")
                            } else {
                                tr("The mod changes aren't known until Sunrise checks this build.")
                            },
                        );
                    }
                }
                Some(VersionAction::Prepare) => {
                    plan = Some(tr("Prepare this build before switching. Your profile stays on its current build."));
                    let target = tag.clone();
                    let b = button("alt-versions-prepare", tr("Prepare"));
                    cache_action = Some(if idle {
                        b.on_click(cx.listener(move |this, _, _, cx| {
                            this.prepare_version(target.clone());
                            cx.notify();
                        }))
                    } else {
                        b.opacity(0.45).tab_stop(false)
                    });
                }
                Some(VersionAction::Download) => {
                    if let Some(release) = self.release_to_install(tag) {
                        plan = Some(tr(
                            "Download this build first. Your profile stays on its current build.",
                        ));
                        let b = button("alt-versions-download", tr("Download"));
                        cache_action = Some(if idle {
                            b.on_click(cx.listener(move |this, _, _, cx| {
                                this.download_for_switch(&release);
                                cx.notify();
                            }))
                        } else {
                            b.opacity(0.45).tab_stop(false)
                        });
                    }
                }
                None => (),
            }
        }
        let keep = pick.as_deref().map(|tag| {
            let keep = crate::frame::button_base("alt-versions-keep", confirm_label(tag, current))
                .bg(rgb(palette().text))
                .border_color(rgb(palette().text))
                .text_color(rgb(palette().background))
                .hover(|s| s.bg(rgb(palette().accent_on)));
            match keep_tag {
                Some(tag) => keep.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.begin_switch(tag.clone(), choice);
                    cx.notify();
                })),
                None => keep.opacity(0.45).cursor_default().tab_stop(false),
            }
        });
        let all = Rc::clone(&list);
        let body = div()
            .flex()
            .flex_col()
            .on_key_down(cx.listener(move |this, ev: &KeyDownEvent, window, cx| {
                let down = match ev.keystroke.key.as_str() {
                    "down" => true,
                    "up" => false,
                    _ => return,
                };
                let v = &mut this.ui.sheets.versions;
                let from = v
                    .rows
                    .iter()
                    .find(|(_, h)| h.is_focused(window))
                    .map(|(n, _)| *n);
                if let Some(to) = next_row(&all, from, down, |_| true) {
                    v.scroll.scroll_to_item(to, gpui::ScrollStrategy::Nearest);
                    let row = v
                        .rows
                        .entry(to)
                        .or_insert_with(|| cx.focus_handle().tab_index(0).tab_stop(true))
                        .clone();
                    window.focus(&row, cx);
                }
                cx.stop_propagation();
                cx.notify();
            }))
            .child(title(tr("Choose a version").into()))
            .child(field)
            .child(
                div()
                    .rounded(crate::tokens::r(crate::tokens::R_CARD))
                    .border_1()
                    .border_color(palette().input_edge())
                    .p(px(4.))
                    .child(rows)
                    .children(nothing),
            )
            .children(
                (!is_pack && !is_update && pick.is_some())
                    .then(|| muted(tr("This profile stays on this build until you change it."))),
            )
            .children(plan.map(muted))
            .children(self.prepare_error.as_ref().map(|e| muted(e.clone())))
            .children(
                self.ui
                    .sheets
                    .error
                    .clone()
                    .or_else(|| {
                        let (tag, error) = self.install_error.as_ref()?;
                        (self.ui.sheets.versions.pick.as_ref() == Some(tag)).then(|| error.clone())
                    })
                    .map(|e| {
                        div()
                            .text_color(rgb(palette().danger))
                            .text_size(px(tokens::SMALL))
                            .child(e)
                    }),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .flex_wrap()
                    .children(reinstall)
                    .child(cancel)
                    .children(cache_action)
                    .children(keep),
            );
        body
    }
}
