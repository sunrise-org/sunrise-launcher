//! "A specific version…": the sheet that picks one build to keep, however many there are.
//!
//! One list, drawn only where it is on screen: what is installed first (the build kept or played
//! leading), then every build that can be downloaded, by channel and month, newest first. The
//! field narrows it by date, name or commit. A row is only chosen; "Keep this build" is what
//! starts the staged Switch. Download and Prepare do not change the playing build.
use super::*;
use crate::line_edit::{self, LineEdit};
use crate::paint::palette;
use std::{
    path::PathBuf,
    rc::Rc,
    sync::{Arc, OnceLock},
};
use sunrise_launcher::{backend::Release, instances::Channel};

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
    preview: Option<FitPreview>,
    /// The list's scroll, which the arrow keys move to the row they land on.
    scroll: gpui::UniformListScrollHandle,
    /// The keyboard focus of each row, by line: a row that is not drawn has no element, so the
    /// handles live here for the arrow keys to land on a row before it is on screen.
    rows: std::collections::HashMap<usize, FocusHandle>,
}

/// The line the Up or Down key lands on from line `from` (None: no row has the keyboard):
/// the nearest build in that direction that can be chosen, or none at the end of the list.
fn next_row(
    lines: &[Line],
    from: Option<usize>,
    down: bool,
    usable: impl Fn(&Build) -> bool,
) -> Option<usize> {
    let ok = |n: &usize| matches!(&lines[*n], Line::Build(b) if usable(b));
    match (from, down) {
        (None, true) => (0..lines.len()).find(ok),
        (None, false) => (0..lines.len()).rev().find(ok),
        (Some(n), true) => (n + 1..lines.len()).find(ok),
        (Some(n), false) => (0..n.min(lines.len())).rev().find(ok),
    }
}

/// A line of the list.
#[derive(Clone, Debug, PartialEq)]
enum Line {
    Head(String),
    Build(Build),
}

#[derive(Clone, Debug, PartialEq)]
struct Build {
    tag: String,
    /// "Nov 17", or a release's own name.
    name: String,
    /// "Nightly · 0c02067", or a release's day.
    about: String,
    installed: bool,
}

#[derive(Debug, PartialEq, Eq)]
enum VersionAction {
    Download,
    Prepare,
    Keep,
}

fn version_action(
    installed: bool,
    prepared: impl FnOnce() -> bool,
    can_switch: bool,
) -> Option<VersionAction> {
    if !installed {
        Some(VersionAction::Download)
    } else if !prepared() {
        Some(VersionAction::Prepare)
    } else {
        can_switch.then_some(VersionAction::Keep)
    }
}

/// "November 2026" for `2026-11`.
fn month(year_month: &str) -> String {
    let (year, month) = year_month.split_once('-').unwrap_or((year_month, ""));
    let name = match month {
        "01" => tr("January"),
        "02" => tr("February"),
        "03" => tr("March"),
        "04" => tr("April"),
        "05" => tr("May"),
        "06" => tr("June"),
        "07" => tr("July"),
        "08" => tr("August"),
        "09" => tr("September"),
        "10" => tr("October"),
        "11" => tr("November"),
        "12" => tr("December"),
        _ => return year_month.to_owned(),
    };
    format!("{name} {year}")
}

fn build(tag: &str, day: &str, installed: bool) -> Build {
    let nightly = sunrise_launcher::nightly::date_of(tag).is_some();
    Build {
        tag: tag.to_owned(),
        name: build_name(tag),
        about: match sunrise_launcher::nightly::label(tag) {
            Some(label) if nightly => label,
            _ => day.to_owned(),
        },
        installed,
    }
}

/// The list for `query`: the installed builds (`current` first), then the builds of `releases`
/// that can be downloaded and aren't installed, under a heading per channel and month.
/// `releases` is newest first, as it is listed. Every word of `query` has to be in a build's
/// version, day or name.
fn lines(
    releases: &[Release],
    installed: &[String],
    current: Option<&str>,
    query: &str,
) -> Vec<Line> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    let day_of = |tag: &str| {
        sunrise_launcher::nightly::date_of(tag)
            .map(str::to_owned)
            .or_else(|| {
                releases
                    .iter()
                    .find(|r| r.tag == tag)
                    .map(|r| r.date.clone())
            })
            .unwrap_or_default()
    };
    let wanted = |b: &Build, day: &str| {
        let hay = format!("{} {} {} {day}", b.tag, b.name, b.about).to_lowercase();
        words.iter().all(|w| hay.contains(w.as_str()))
    };
    // Launcher releases come from their own repository and are never installed here, and `v2026.10.13` is also valid vX.Y.Z: only native builds are left out.
    let game = |tag: &str| !native_mods::is_native(tag);
    let mut out = Vec::new();
    let mut have: Vec<&String> = installed.iter().filter(|t| game(t)).collect();
    have.sort_by(|a, b| {
        (Some(b.as_str()) == current)
            .cmp(&(Some(a.as_str()) == current))
            .then_with(|| sunrise_launcher::backend::compare_versions(b, a))
    });
    let have: Vec<Line> = have
        .into_iter()
        .map(|tag| (build(tag, &day_of(tag), true), day_of(tag)))
        .filter(|(b, day)| wanted(b, day))
        .map(|(b, _)| Line::Build(b))
        .collect();
    if !have.is_empty() {
        out.push(Line::Head(tr("Installed").to_string()));
        out.extend(have);
    }
    // A Nightly can be listed under two names that install as one version: it is one build.
    let mut seen = std::collections::HashSet::new();
    for (channel, title) in [
        (Some(Channel::Nightly), tr("Nightly")),
        (Some(Channel::Stable), tr("Stable")),
        (None, tr("Pre-release")),
    ] {
        let mut heading = String::new();
        for r in releases.iter().filter(|r| {
            r.asset.is_some()
                && Channel::of_release(r) == channel
                && (channel.is_some() || backend::is_game_tag(&r.tag))
                && !installed.contains(&r.tag)
                && seen.insert(r.tag.as_str())
        }) {
            let day = day_of(&r.tag);
            let b = build(&r.tag, &day, false);
            if !wanted(&b, &day) {
                continue;
            }
            let under = format!("{title} · {}", month(day.get(..7).unwrap_or(&day)));
            if under != heading {
                out.push(Line::Head(under.clone()));
                heading = under;
            }
            out.push(Line::Build(b));
        }
    }
    out
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
        v.preview = None;
        v.rows.clear();
    }

    /// Reads whether the field has the keyboard, before the sheet is drawn.
    pub(crate) fn versions_focus(&mut self, window: &Window) {
        let v = &mut self.ui.sheets.versions;
        v.focused = v.focus.as_ref().is_some_and(|f| f.is_focused(window));
    }

    pub(super) fn versions_sheet(&mut self, cx: &mut Context<Self>) -> Stateful<Div> {
        let body = self.versions_content(true, cx);
        frame("alt-versions", 560., body, cx)
    }

    pub(crate) fn open_update_picker(&mut self, tag: String) {
        self.open_versions();
        self.ui.sheets.versions.pick = Some(tag);
    }

    pub(crate) fn versions_page(&mut self, cx: &mut Context<Self>) -> Stateful<Div> {
        self.versions_content(false, cx)
            .id("versions-page")
            .flex_1()
            .min_h_0()
            .p(px(24.))
            .gap(px(12.))
            .overflow_y_scroll()
            .track_scroll(&crate::frame::scrolls("versions-page"))
    }

    fn versions_content(&mut self, sheet: bool, cx: &mut Context<Self>) -> Div {
        let Some(instance) = self.selected_instance() else {
            return div().child(tr("No profile selected."));
        };
        let plays = self.profile_target(instance);
        let is_pack = instance.kind == instances::Kind::Pack;
        let home = instance.home.clone();
        let releases: &[Release] = match &self.releases {
            Load::Ready(rows) => rows,
            _ => &[],
        };
        let current = plays.as_deref();
        let query = self.ui.sheets.versions.find.text().to_owned();
        let list = Rc::new(lines(releases, &self.installed, current, &query));
        let pick = self.ui.sheets.versions.pick.clone();

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
                "Search by date or commit",
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
        // As tall as what it lists, up to the room the window has; past that it scrolls.
        .h(px((list.len() as f32 * LINE_H)
            .min((self.window_height - 330.).clamp(LINE_H * 4., LINE_H * 9.))));
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
        let keep = crate::frame::button_base("alt-versions-keep", tr("Keep this build"))
            .bg(rgb(palette().text))
            .border_color(rgb(palette().text))
            .text_color(rgb(palette().background))
            .hover(|s| s.bg(rgb(palette().accent_on)));
        let idle = !is_pack && !self.launch_busy() && self.client.is_none();
        let mut cache_action = None;
        let mut plan = None;
        let mut keep_tag = None;
        if let Some(tag) = pick.as_ref() {
            match version_action(
                self.installed.contains(tag),
                || backend::is_prepared(tag),
                sheet && !is_pack && Some(tag.as_str()) != current,
            ) {
                Some(VersionAction::Keep) => {
                    if idle {
                        keep_tag = Some(tag.clone());
                    }
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
                            sunrise_launcher::browser::sunrise::checked_switch_fit(&home, &target)
                        });
                        cx.spawn(async move |this, cx| {
                            let _ = checked.set(work.await);
                            let _ = this.update(cx, |_, cx| cx.notify());
                        })
                        .detach();
                    }
                    plan = Some(
                        if request.as_ref().and_then(|p| p.checked.get()) == Some(&true) {
                            tr("The installed mod files fit this build.")
                        } else {
                            tr("The mod changes aren't known until Sunrise checks this build.")
                        },
                    );
                }
                Some(VersionAction::Prepare) => {
                    plan = Some(tr("Prepare this build before switching. Your profile stays on its current build."));
                    let target = tag.clone();
                    let b = button("alt-versions-prepare", tr("Prepare"));
                    cache_action = Some(if idle {
                        b.on_click(cx.listener(move |this, _, _, cx| {
                            this.prepare_version(target.clone(), backend::MinecraftFrom::Install);
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
        let keep = match keep_tag {
            Some(tag) => keep.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.begin_switch(tag.clone());
                cx.notify();
            })),
            None => keep.opacity(0.45).cursor_default().tab_stop(false),
        };
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
                    .border_color(rgb(palette().line))
                    .p(px(4.))
                    .child(rows)
                    .children(nothing),
            )
            .children(plan.map(muted))
            .children(self.prepare_error.as_ref().map(|e| muted(e.clone())))
            .children(self.ui.sheets.error.clone().map(|e| {
                div()
                    .text_color(rgb(palette().danger))
                    .text_size(px(tokens::SMALL))
                    .child(e)
            }))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .children(sheet.then_some(cancel))
                    .children(cache_action)
                    .children(sheet.then_some(keep)),
            );
        body
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_current_unready_build_can_be_downloaded_or_prepared_but_never_kept() {
        assert_eq!(
            version_action(false, || unreachable!(), false),
            Some(VersionAction::Download)
        );
        assert_eq!(
            version_action(true, || false, false),
            Some(VersionAction::Prepare)
        );
        assert_eq!(version_action(true, || true, false), None);
        assert_eq!(
            version_action(true, || true, true),
            Some(VersionAction::Keep)
        );
    }

    fn release(tag: &str, date: &str, nightly: bool) -> Release {
        Release {
            tag: tag.into(),
            name: tag.into(),
            body: String::new(),
            date: date.into(),
            time: String::new(),
            asset: Some((String::new(), 1)),
            prerelease: false,
            nightly: nightly.then(|| "nightly".to_owned()),
        }
    }

    fn build(tag: &str) -> Line {
        Line::Build(Build {
            tag: tag.into(),
            name: tag.into(),
            about: String::new(),
            installed: false,
        })
    }

    #[test]
    fn arrow_keys_step_over_headings_and_unusable_rows_and_stop_at_the_ends() {
        let lines = [
            Line::Head("November".into()),
            build("a"),
            build("b"),
            Line::Head("October".into()),
            build("c"),
            build("d"),
        ];
        let all = |_: &Build| true;
        assert_eq!(next_row(&lines, None, true, all), Some(1));
        assert_eq!(next_row(&lines, None, false, all), Some(5));
        assert_eq!(next_row(&lines, Some(2), true, all), Some(4));
        assert_eq!(next_row(&lines, Some(4), false, all), Some(2));
        assert_eq!(next_row(&lines, Some(5), true, all), None);
        assert_eq!(next_row(&lines, Some(1), false, all), None);
        let not_c = |b: &Build| b.tag != "c";
        assert_eq!(next_row(&lines, Some(2), true, not_c), Some(5));
        assert_eq!(next_row(&[], None, true, all), None);
    }

    fn shape(lines: &[Line]) -> Vec<String> {
        lines
            .iter()
            .map(|line| match line {
                Line::Head(text) => format!("# {text}"),
                Line::Build(b) => b.tag.clone(),
            })
            .collect()
    }

    #[test]
    fn a_rolling_nightly_and_an_old_prerelease_never_have_a_stable_heading() {
        let mut releases = vec![
            release("nightly-2026-11-17-aaaaaaa", "2026-11-17", true),
            Release {
                prerelease: true,
                ..release("v1.1.0", "2026-11-16", false)
            },
        ];
        assert_eq!(
            shape(&lines(&releases, &[], None, "")),
            [
                "# Nightly · November 2026",
                "nightly-2026-11-17-aaaaaaa",
                "# Pre-release · November 2026",
                "v1.1.0",
            ]
        );
        releases.push(release("v1.0.0", "2026-10-01", false));
        assert_eq!(
            shape(&lines(&releases, &[], None, "v1.0.0")),
            ["# Stable · October 2026", "v1.0.0"]
        );
    }

    #[test]
    fn installed_builds_lead_then_the_rest_by_channel_and_month() {
        let releases = [
            release("nightly-2026-11-17-aaaaaaa", "2026-11-17", true),
            release("nightly-2026-11-16-bbbbbbb", "2026-11-16", true),
            release("nightly-2026-10-30-ccccccc", "2026-10-30", true),
            // Listed a second time under its dated name.
            release("nightly-2026-10-30-ccccccc", "2026-10-30", true),
            release("v1.0.0", "2026-10-01", false),
        ];
        let installed = [
            "nightly-2026-11-17-aaaaaaa".to_owned(),
            "nightly-2026-11-16-bbbbbbb".to_owned(),
        ];
        let all = lines(
            &releases,
            &installed,
            Some("nightly-2026-11-16-bbbbbbb"),
            "",
        );
        assert_eq!(
            shape(&all),
            [
                "# Installed",
                // The build kept or played leads, whatever its date.
                "nightly-2026-11-16-bbbbbbb",
                "nightly-2026-11-17-aaaaaaa",
                "# Nightly · October 2026",
                "nightly-2026-10-30-ccccccc",
                "# Stable · October 2026",
                "v1.0.0",
            ]
        );
        // A commit or a day finds its build; a heading with nothing under it is gone.
        let found = lines(&releases, &installed, None, "ccccccc");
        assert_eq!(
            shape(&found),
            ["# Nightly · October 2026", "nightly-2026-10-30-ccccccc"]
        );
        assert_eq!(
            shape(&lines(&releases, &installed, None, "nov 17")).len(),
            2
        );
    }
}
