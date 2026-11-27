//! "A specific version…": the sheet that picks one build to keep, however many there are.
//!
//! One list, drawn only where it is on screen: what is installed first (the build kept or played
//! leading), then every build that can be downloaded, by channel and month, newest first. The
//! field narrows it by date, name or commit. A row only chooses; confirming starts the
//! staged switch, which downloads and prepares the build first when it needs that. Download and
//! Prepare are offered only for the build the profile is on already.
use super::*;
use crate::line_edit::{self, LineEdit};
use crate::paint::palette;
use std::{
    path::PathBuf,
    rc::Rc,
    sync::{Arc, OnceLock},
};
use sunrise_launcher::{backend::Release, browser::sunrise::BuildChoice, instances::Channel};

/// The height of every line of the list, headings included.
const LINE_H: f32 = 40.;
struct FitPreview {
    home: PathBuf,
    tag: String,
    /// The inventory Home had read when it was asked: a change of mods, pins or holds asks
    /// again.
    inventory: u64,
    /// The signed mod index loaded when it was asked: a newer index asks again.
    index: Option<u64>,
    /// When the check finished: retry/recheck age and the background timer both count from
    /// completion so a slow failure still pauses before retrying.
    completed: Option<Instant>,
    /// Set by the periodic refresh timer once it fires for this check.
    due: bool,
    /// What keeps the profile's wanted mods from this build; the inner None when the index or
    /// the mods couldn't be read, which is asked again after a while.
    checked: Arc<OnceLock<Option<Vec<sunrise_launcher::browser::sunrise::Blocker>>>>,
}

impl FitPreview {
    fn needs_refresh(
        &self,
        home: &std::path::Path,
        tag: &str,
        inventory: u64,
        index: Option<u64>,
        now: Instant,
    ) -> bool {
        self.home != home
            || self.tag != tag
            || self.inventory != inventory
            || self.index != index
            || self.due
            || match (self.checked.get(), self.completed) {
                (Some(None), Some(completed)) => {
                    now.saturating_duration_since(completed) >= PREVIEW_RETRY
                }
                (Some(Some(_)), Some(completed)) => {
                    now.saturating_duration_since(completed) >= PREVIEW_RECHECK
                }
                _ => false,
            }
    }
}

/// A failed preview is asked again after this long, a good one after the longer: the signed
/// index changes on its own.
const PREVIEW_RETRY: std::time::Duration = std::time::Duration::from_secs(30);
const PREVIEW_RECHECK: std::time::Duration = std::time::Duration::from_secs(5 * 60);

/// The picker's sentence on what a build leaves out, by what is known of each mod.
fn preview_plan(blockers: &[sunrise_launcher::browser::sunrise::Blocker]) -> String {
    use sunrise_launcher::browser::sunrise::FitIssue;
    if blockers.is_empty() {
        return tr("Every mod in this profile has a release for this build.").into();
    }
    let of = |issue: FitIssue| {
        blockers
            .iter()
            .filter(|blocker| blocker.issue == issue)
            .map(|blocker| blocker.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    [
        (FitIssue::NoRelease, "No release for this build: {mods}."),
        (FitIssue::Unproved, "Not marked for this build yet: {mods}."),
        (
            FitIssue::NotFromRegistry,
            "Not from the registry, so not checked: {mods}.",
        ),
        (FitIssue::Withdrawn, "Withdrawn from the registry: {mods}."),
    ]
    .into_iter()
    .filter_map(|(issue, text)| {
        let mods = of(issue);
        (!mods.is_empty()).then(|| trf(text, &[("mods", &mods)]))
    })
    .collect::<Vec<_>>()
    .join(" ")
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
    Choose,
}

fn confirm_label(tag: &str, current: Option<&str>) -> String {
    if Some(tag) == current {
        tr("Keep this build").into()
    } else {
        trf("Switch to {build}", &[("build", &build_name(tag))])
    }
}

fn show_architecture_reinstall(
    tag: &str,
    repair_available: bool,
    architecture_reinstall: Option<&str>,
    installing: Option<&(String, backend::InstallPhase)>,
) -> bool {
    repair_available
        || (architecture_reinstall == Some(tag)
            && installing.is_some_and(|(started, _)| started == tag))
}

fn reinstall_label(
    tag: &str,
    installing: Option<&(String, backend::InstallPhase)>,
    detail: Option<&str>,
) -> String {
    let Some((_, phase)) = installing.filter(|(started, _)| started == tag) else {
        return tr("Reinstall for this architecture").into();
    };
    if let (backend::InstallPhase::Preparing, Some(detail)) = (*phase, detail) {
        return detail.to_owned();
    }
    crate::mods_page::install::stage_label((*phase).into())
}

/// What the sheet offers for the picked build. Another build the profile can switch to is
/// chosen in one press, which downloads and prepares it first when it needs that: a pick that
/// only downloaded left the profile on its old build, still "the latest" (a player's report,
/// 2026-11-26). The build the profile plays (`current`) is downloaded or prepared when it isn't
/// ready, as keeping it only saves the choice, and kept once it is.
fn version_action(
    installed: bool,
    prepared: impl FnOnce() -> bool,
    can_switch: bool,
    current: bool,
) -> Option<VersionAction> {
    if can_switch && !current {
        Some(VersionAction::Choose)
    } else if !installed {
        Some(VersionAction::Download)
    } else if !prepared() {
        Some(VersionAction::Prepare)
    } else {
        can_switch.then_some(VersionAction::Choose)
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
    Build {
        tag: tag.to_owned(),
        name: build_name(tag),
        about: sunrise_launcher::nightly::date_of(tag)
            .or_else(|| backend::dev_date(tag))
            .and_then(|_| tag.splitn(5, '-').nth(4))
            .map_or_else(|| day.to_owned(), sunrise_launcher::nightly::short_commit),
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
    channel: Channel,
    query: &str,
) -> Vec<Line> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    let day_of = |tag: &str| {
        sunrise_launcher::nightly::date_of(tag)
            .or_else(|| backend::dev_date(tag))
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
    // Launcher releases are separate. Dev is opt-in, but keep a profile's exact current build
    // visible even if it has another saved channel (for example, a copy of a Dev profile).
    let game = |tag: &str| {
        !native_mods::is_native(tag)
            && (channel == Channel::Dev || Some(tag) == current || backend::dev_date(tag).is_none())
    };
    let mut out = Vec::new();
    let mut have: Vec<&String> = installed.iter().filter(|t| game(t)).collect();
    have.sort_by(|a, b| {
        (Some(b.as_str()) == current)
            .cmp(&(Some(a.as_str()) == current))
            .then_with(|| sunrise_launcher::backend::compare_game_versions(b, a))
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
    for (listed, title) in [
        (Channel::Nightly, tr("Nightly")),
        (Channel::Stable, tr("Stable")),
        (Channel::Dev, tr("Dev")),
    ] {
        if listed == Channel::Dev && channel != Channel::Dev {
            continue;
        }
        let mut heading = String::new();
        for r in releases.iter().filter(|r| {
            r.asset.is_some()
                && Channel::of_release(r) == Some(listed)
                && !installed.contains(&r.tag)
        }) {
            let day = day_of(&r.tag);
            let b = build(&r.tag, &day, false);
            if !wanted(&b, &day) {
                continue;
            }
            let under = trf(
                "{channel} · {month}",
                &[
                    ("channel", title),
                    ("month", &month(day.get(..7).unwrap_or(&day))),
                ],
            );
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
        let body = self.versions_content(cx);
        frame("alt-versions", 560., body, cx)
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

    /// Every build `instance` could switch to besides `plays` (the picker's list), to find one
    /// its mods have releases for: the profile's channel first, then the others, each newest
    /// first. All are checked, so "no build has every mod" is true of the whole list.
    pub(crate) fn switch_candidates(
        &self,
        instance: &instances::Instance,
        plays: Option<&str>,
    ) -> Vec<String> {
        let releases: &[Release] = match &self.releases {
            Load::Ready(rows) => rows,
            _ => &[],
        };
        let mut tags: Vec<String> = lines(releases, &self.installed, plays, instance.channel, "")
            .into_iter()
            // The build the profile plays isn't known here without reading its pointer: the
            // caller drops it (`plays` only places the list as the picker does).
            .filter_map(|line| match line {
                Line::Build(b) => Some(b.tag),
                _ => None,
            })
            .collect();
        let elsewhere = |tag: &str| channel_build(tag).map(|(c, _)| c) != Some(instance.channel);
        tags.sort_by(|a, b| {
            elsewhere(a)
                .cmp(&elsewhere(b))
                .then_with(|| sunrise_launcher::backend::compare_game_versions(b, a))
        });
        tags
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
        // Picked by hand, so it is kept: Update game moves a following profile on by itself.
        let choice = Some(BuildChoice::Keep);
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
            let offered = self.architecture_repair(tag).is_some();
            if !show_architecture_reinstall(
                tag,
                offered,
                self.architecture_reinstall.as_deref(),
                self.installing.as_ref(),
            ) {
                return None;
            }
            let tag = tag.clone();
            let button = button(
                "alt-versions-reinstall",
                reinstall_label(&tag, self.installing.as_ref(), self.play_detail.as_deref()),
            );
            Some(if offered && self.can_repair_now(&tag) {
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
        let mut plan: Option<SharedString> = None;
        let mut keep_tag = None;
        let mut ready_note = None;
        if let Some(tag) = pick.as_ref() {
            let installed = self.installed.contains(tag);
            match version_action(
                installed,
                || backend::is_prepared(tag),
                !(is_pack || already_kept && Some(tag.as_str()) == current),
                Some(tag.as_str()) == current,
            ) {
                Some(VersionAction::Choose) => {
                    let ready = installed && backend::is_prepared(tag);
                    if idle && (installed || self.release_to_install(tag).is_some()) {
                        keep_tag = Some(tag.clone());
                    }
                    if !ready {
                        ready_note = Some(if installed {
                            tr("Switching prepares this build first.")
                        } else {
                            tr("Switching downloads and prepares this build first.")
                        });
                    }
                    if Some(tag.as_str()) != current {
                        let inventory = self.ui.home.mods.revision();
                        let index = sunrise_launcher::browser::sunrise::v2::loaded_serial();
                        let now = Instant::now();
                        let request = &mut self.ui.sheets.versions.preview;
                        if request.as_ref().is_none_or(|preview| {
                            preview.needs_refresh(&home, tag, inventory, index, now)
                        }) {
                            let checked = Arc::new(OnceLock::new());
                            *request = Some(FitPreview {
                                home: home.clone(),
                                tag: tag.clone(),
                                inventory,
                                index,
                                completed: None,
                                due: false,
                                checked: checked.clone(),
                            });
                            let target = tag.clone();
                            let work = cx.background_executor().spawn(async move {
                                match sunrise_launcher::browser::sunrise::v2::index() {
                                    Ok(catalog) => {
                                        let blockers =
                                            sunrise_launcher::browser::sunrise::switch_fit_blockers_v2(
                                                &catalog.index,
                                                &home,
                                                &target,
                                            );
                                        (blockers.as_ref().map(|_| catalog.index.serial), blockers)
                                    }
                                    Err(_) => (None, None),
                                }
                            });
                            cx.spawn(async move |this, cx| {
                                let (used_serial, result) = work.await;
                                let delay = if result.is_none() {
                                    PREVIEW_RETRY
                                } else {
                                    PREVIEW_RECHECK
                                };
                                let _ = checked.set(result);
                                let finished = Instant::now();
                                let active = this
                                    .update(cx, |this, cx| {
                                        let Some(preview) =
                                            this.ui.sheets.versions.preview.as_mut()
                                        else {
                                            return false;
                                        };
                                        if !Arc::ptr_eq(&preview.checked, &checked) {
                                            return false;
                                        }
                                        preview.completed = Some(finished);
                                        if let Some(serial) = used_serial {
                                            preview.index = Some(serial);
                                        }
                                        cx.notify();
                                        true
                                    })
                                    .unwrap_or(false);
                                if !active {
                                    return;
                                }
                                cx.background_executor().timer(delay).await;
                                let _ = this.update(cx, |this, cx| {
                                    if let Some(preview) = this.ui.sheets.versions.preview.as_mut()
                                    {
                                        if Arc::ptr_eq(&preview.checked, &checked) {
                                            preview.due = true;
                                            cx.notify();
                                        }
                                    }
                                });
                            })
                            .detach();
                        }
                        plan = Some(
                            match request.as_ref().and_then(|p| p.checked.get()) {
                                // Which mods this build leaves out and why, so the pick is the
                                // player's.
                                Some(Some(blockers)) => preview_plan(blockers),
                                _ => tr(
                                    "The mod changes aren't known until Sunrise checks this build.",
                                )
                                .to_string(),
                            }
                            .into(),
                        );
                    }
                }
                Some(VersionAction::Prepare) => {
                    plan = Some(tr("Prepare this build before switching. Your profile stays on its current build.").into());
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
                        )
                        .into());
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
                    .border_color(rgb(palette().divider))
                    .p(px(4.))
                    .child(rows)
                    .children(nothing),
            )
            .children(
                (!is_pack && pick.is_some())
                    .then(|| muted(tr("This profile stays on this build until you change it."))),
            )
            .children(plan.map(muted))
            .children(ready_note.map(muted))
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

impl Launcher {
    /// The build list as `sunrise-launcher benchmark` finds it after a frame, and the bounds a
    /// wheel step goes over. Read between frames, never while one is built: a read inside one
    /// would make the list a dependency of the launcher's view and turn off the scroll-only
    /// reuse a player's wheel gets.
    ///
    /// Once `timed`, the rows are known to be drawn and only the list's place is read: working
    /// out the rows again (the profile's build is read from disk) would add work to every
    /// measured frame that a player's wheel doesn't cause.
    fn bench_target(&self, timed: bool) -> (perf::Target, gpui::Bounds<gpui::Pixels>) {
        use perf::Target;
        let scroll = self.ui.sheets.versions.scroll.0.borrow();
        let bounds = scroll.base_handle.bounds();
        let laid = || Target::Laid {
            offset: f32::from(scroll.base_handle.offset().y),
            max: f32::from(scroll.base_handle.max_offset().y),
        };
        let target = if self.ui.sheet != Some(Sheet::Versions) {
            Target::Closed
        } else if timed {
            laid()
        } else if let Some(instance) = self.selected_instance() {
            if matches!(self.releases, Load::Loading) {
                Target::Loading
            } else {
                let plays = self.profile_target(instance);
                let rows = self.version_lines(instance, plays.as_deref()).len();
                // The last frame laid the list out with every loaded row, at its height: not
                // the short list drawn before the releases arrived.
                let drawn = scroll.last_item_size.is_some_and(|size| {
                    (f32::from(size.contents.height) - rows as f32 * LINE_H).abs() < 0.5
                        && (f32::from(bounds.size.height) - self.versions_list_height(rows)).abs()
                            < 0.5
                });
                match (rows, drawn) {
                    (0, _) => Target::Empty,
                    (_, false) => Target::LayingOut,
                    (_, true) => laid(),
                }
            }
        } else {
            Target::NoProfile
        };
        (target, bounds)
    }
}

/// `sunrise-launcher benchmark`: after each frame, looks at the list it scrolls (the build
/// picker's, or Installed's: [`perf::Bench::list`]) and sends one wheel step over it through the
/// window's ordinary input path, so the list scrolls, caches and redraws as it does under a
/// player's wheel. Nothing is read inside a frame.
///
/// It keeps looking while the list loads and is laid out with the rows it brings; the
/// benchmark's timed phase only begins once they are drawn ([`perf::Bench::after_frame`]). With
/// no profile, no rows, a list that couldn't be read, or rows that fit, the phase ends there and
/// no more frames are asked for.
pub(crate) fn bench_wheel(launcher: gpui::WeakEntity<Launcher>, window: &mut Window) {
    window.on_next_frame(move |window, cx| {
        let Some(entity) = launcher.upgrade() else {
            return;
        };
        let this = entity.read(cx);
        let timed = this.bench.as_ref().is_some_and(perf::Bench::timing_scroll);
        let (target, bounds) = match this.bench.as_ref().map(perf::Bench::list) {
            Some(perf::List::Installed) => this.installed_bench_target(),
            _ => this.bench_target(timed),
        };
        let now = std::time::Instant::now();
        let step = entity.update(cx, |l, _| l.bench.as_mut()?.after_frame(now, target));
        if let Some(step) = step {
            window.dispatch_event(bench_wheel_event(bounds, step), cx);
        }
        let scrolling = entity
            .read(cx)
            .bench
            .as_ref()
            .is_some_and(perf::Bench::scrolling);
        if scrolling {
            bench_wheel(launcher, window);
        }
    });
}

/// The wheel event a benchmark sends: `step` pixels down (negative) or up, at the middle of
/// `bounds`.
fn bench_wheel_event(bounds: gpui::Bounds<gpui::Pixels>, step: f32) -> gpui::PlatformInput {
    gpui::PlatformInput::Scroll(gpui::ScrollEvent {
        position: bounds.center(),
        delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(step))),
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn architecture_reinstall_reports_only_the_picked_builds_phase() {
        use backend::InstallPhase;
        let tag = "26.3-0.1.0";
        let active = |phase| (tag.to_owned(), phase);
        let detail = "Copying the prepared game files: 2 of 8";
        for (phase, label) in [
            (InstallPhase::Checking, "Checking the build…"),
            (InstallPhase::Downloading(0.25), "Downloading the build 25%"),
            (InstallPhase::Unpacking, "Unpacking the build"),
            (InstallPhase::Finishing, "Putting the build in place"),
            (InstallPhase::Preparing, "Getting the build ready…"),
        ] {
            let installing = active(phase);
            assert_eq!(reinstall_label(tag, Some(&installing), None), label);
            assert_eq!(
                reinstall_label("other", Some(&installing), Some(detail)),
                "Reinstall for this architecture"
            );
        }
        assert_eq!(
            reinstall_label(tag, Some(&active(InstallPhase::Preparing)), Some(detail)),
            detail
        );
    }

    #[test]
    fn architecture_reinstall_button_stays_visible_while_donor_prep_finishes() {
        use backend::InstallPhase;
        let tag = "26.3-0.1.0";
        let preparing = (tag.to_owned(), InstallPhase::Preparing);
        // Once the new binary lands on disk, `architecture_repair` becomes None while donor
        // preparation still runs under `InstallPhase::Preparing`; keep the progress button visible.
        assert!(show_architecture_reinstall(
            tag,
            false,
            Some(tag),
            Some(&preparing),
        ));
        assert!(!show_architecture_reinstall(
            "other",
            false,
            Some(tag),
            Some(&preparing),
        ));
        assert!(!show_architecture_reinstall(
            tag,
            false,
            None,
            Some(&preparing),
        ));
        assert!(!show_architecture_reinstall(tag, false, Some(tag), None));
        assert!(show_architecture_reinstall(tag, true, None, None));
    }

    #[test]
    fn confirmation_names_the_picked_build_only_when_switching() {
        let current = "dev-2026-11-22-7cfdf81";
        let picked = "nightly-2026-11-20-aaaaaaa";
        assert_eq!(confirm_label(current, Some(current)), "Keep this build");
        assert_eq!(
            confirm_label(picked, Some(current)),
            "Switch to Nightly · Nov 20"
        );
    }

    /// `sunrise-launcher benchmark`'s scroll phase in a real launcher window: the pump's tick
    /// opens the build picker and schedules the wheel (`Launcher::drain`), the release list
    /// arrives only after that, as on a cold start, and the timing waits for it. Then the
    /// benchmark's own wheel events move the populated list, and the frames that moved it are
    /// its samples.
    ///
    /// Runs in a child of the test binary with scratch homes, no keychain and an unreachable
    /// proxy, so the window it opens reads and writes no player's data and fetches nothing.
    #[test]
    fn the_benchmark_scrolls_the_populated_build_picker_once_its_rows_arrive() {
        const CHILD: &str = "SUNRISE_BENCH_PICKER_TEST";
        let Some(root) = std::env::var_os(CHILD).map(PathBuf::from) else {
            let scratch = tempfile::tempdir().unwrap();
            let root = scratch.path();
            let nowhere = "http://127.0.0.1:9";
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "sheets::versions::tests::the_benchmark_scrolls_the_populated_build_picker_once_its_rows_arrive",
                    "--nocapture",
                ])
                .env(CHILD, root)
                .env("SUNRISE_LAUNCHER_HOME", root.join("launcher"))
                .env("SUNRISE_HOME", root.join("game"))
                .env("SUNRISE_LAUNCHER_LOCATION_FILE", root.join("location.json"))
                .env("SUNRISE_NO_KEYCHAIN", "1")
                .env("SUNRISE_TELEMETRY_URL", "off")
                .env("SUNRISE_LAUNCHER_NO_SELF_UPDATE", "1")
                .env("XDG_DATA_HOME", root.join("data"))
                .env("XDG_CONFIG_HOME", root.join("config"))
                .env("HTTPS_PROXY", nowhere)
                .env("HTTP_PROXY", nowhere)
                .env("ALL_PROXY", nowhere)
                .env("https_proxy", nowhere)
                .env("http_proxy", nowhere)
                .env("all_proxy", nowhere)
                .env_remove("NO_PROXY")
                .env_remove("no_proxy")
                .env_remove("SUNRISE_GITHUB_MIRROR")
                .env_remove("SUNRISE_LAUNCHER_BENCH")
                .status()
                .unwrap();
            assert!(status.success());
            return;
        };
        let mut app = gpui::TestAppContext::single();
        // The window as the launcher opens it: the launcher drawn as a cached view of its root.
        let (shell, cx) = app.add_window_view(crate::window_root::Shell::new_for_test);
        let launcher = shell.read_with(cx, |shell, _| shell.launcher());
        let handle = cx.update(|window, _| window.window_handle());
        let out = root.join("bench.json");
        launcher.update(cx, |l, _| {
            assert!(
                l.selected_instance().is_some(),
                "a fresh home has a profile"
            );
            // A cold start: the picker opens while the release list is still on its way.
            l.releases = Load::Loading;
            l.bench = Some(crate::perf::Bench::switched_for_test(
                out.clone(),
                crate::perf::List::Builds,
            ));
        });
        // The tick that opens the picker and schedules the wheel, as the pump runs it.
        launcher.update(cx, |l, cx| l.drain(handle, cx));
        cx.run_until_parked();
        let read = |cx: &mut gpui::VisualTestContext| {
            launcher.read_with(cx, |l, _| {
                let bench = l.bench.as_ref().unwrap();
                let list = &l.ui.sheets.versions.scroll.0.borrow().base_handle;
                (
                    bench.scrolling(),
                    bench.timing_scroll(),
                    bench.scroll_samples().len(),
                    f32::from(list.offset().y),
                )
            })
        };
        // A frame from the platform: the wheel's callbacks run, then the window draws.
        let frame = |cx: &mut gpui::VisualTestContext| {
            let ran = cx.update(|window, cx| window.simulate_next_frame(cx));
            cx.run_until_parked();
            ran
        };
        assert_eq!(
            launcher.read_with(cx, |l, _| l.ui.sheet),
            Some(Sheet::Versions)
        );
        for _ in 0..5 {
            assert_eq!(frame(cx), 1, "the wheel looks after every frame");
            let (scrolling, timed, samples, offset) = read(cx);
            assert!(scrolling && !timed, "waiting for the release list, untimed");
            assert_eq!((samples, offset), (0, 0.));
        }
        // The releases arrive, as the pump hands them over.
        let releases: Vec<Release> = (1..=28)
            .flat_map(|day| {
                [10, 11].map(|month| {
                    let date = format!("2026-{month}-{day:02}");
                    release(&format!("nightly-{date}-aaaaaaa"), &date, true)
                })
            })
            .collect();
        launcher.update(cx, |l, _| l.releases = Load::Ready(releases));
        // The platform's next frame runs the callbacks before it draws: the list is loaded
        // but still laid out empty, which is neither a list that fits nor the timing's start.
        frame(cx);
        let (scrolling, timed, _, _) = read(cx);
        assert!(
            scrolling && !timed,
            "waiting for the loaded rows to be drawn"
        );
        launcher.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
        // The frame after the one that drew the long list: from here the phase is timed.
        frame(cx);
        let (scrolling, timed, samples, offset) = read(cx);
        assert!(scrolling && timed, "timed once the rows are drawn");
        assert_eq!(samples, 0, "drawing the rows is no scroll sample");
        assert!(offset < 0., "the first wheel step moved the list: {offset}");
        let mut last = offset;
        for _ in 0..3 {
            frame(cx);
            let (_, _, _, offset) = read(cx);
            assert!(
                offset < last,
                "the list keeps moving down: {offset} after {last}"
            );
            last = offset;
        }
        let (_, _, samples, _) = read(cx);
        assert!(
            samples >= 3,
            "each frame that moved the list is a sample: {samples}"
        );
    }

    #[test]
    fn the_current_unready_build_can_be_downloaded_or_prepared_but_never_kept() {
        // A kept profile (no switch to offer) and a following one alike: keeping the build it
        // plays only saves the choice, so an unready one is readied first (review of #616).
        for can_switch in [false, true] {
            assert_eq!(
                version_action(false, || unreachable!(), can_switch, true),
                Some(VersionAction::Download)
            );
            assert_eq!(
                version_action(true, || false, can_switch, true),
                Some(VersionAction::Prepare)
            );
        }
        assert_eq!(version_action(true, || true, false, true), None);
        assert_eq!(
            version_action(true, || true, true, true),
            Some(VersionAction::Choose)
        );
    }

    /// Another picked build is switched to in one press however far it is from ready: a pick
    /// that only downloaded left the profile on its old build (a player's report, 2026-11-26).
    #[test]
    fn a_picked_build_is_switched_to_in_one_press_even_before_it_is_downloaded() {
        assert_eq!(
            version_action(false, || unreachable!(), true, false),
            Some(VersionAction::Choose)
        );
        assert_eq!(
            version_action(true, || unreachable!(), true, false),
            Some(VersionAction::Choose)
        );
    }

    #[test]
    fn the_preview_says_what_is_known_of_each_mod() {
        use sunrise_launcher::browser::sunrise::{Blocker, FitIssue};
        let blocker = |name: &str, issue| Blocker {
            name: name.into(),
            issue,
        };
        assert_eq!(
            preview_plan(&[]),
            "Every mod in this profile has a release for this build."
        );
        assert_eq!(
            preview_plan(&[
                blocker("Wayfinder", FitIssue::NoRelease),
                blocker("Minimap", FitIssue::Unproved),
                blocker("Lanterns", FitIssue::Unproved),
            ]),
            "No release for this build: Wayfinder. Not marked for this build yet: Minimap, Lanterns."
        );
    }

    fn release(tag: &str, date: &str, nightly: bool) -> Release {
        Release {
            tag: tag.into(),
            name: tag.into(),
            body: String::new(),
            date: date.into(),
            time: String::new(),
            published_at: String::new(),
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
    fn only_current_nightly_and_stable_builds_have_published_rows() {
        let mut releases = vec![
            release("nightly-2026-11-17-aaaaaaa", "2026-11-17", true),
            Release {
                prerelease: true,
                ..release("26.3-0.1.1", "2026-11-16", false)
            },
        ];
        assert_eq!(
            shape(&lines(&releases, &[], None, Channel::Nightly, "")),
            ["# Nightly · November 2026", "nightly-2026-11-17-aaaaaaa",]
        );
        releases.push(release("26.3-0.1.0", "2026-10-01", false));
        assert_eq!(
            shape(&lines(&releases, &[], None, Channel::Nightly, "26.3-0.1.0")),
            ["# Stable · October 2026", "26.3-0.1.0"]
        );
    }

    #[test]
    fn dev_versions_are_listed_only_after_opt_in() {
        let tag = "dev-2026-11-21-aaaaaaa";
        let pruned = "dev-2026-11-20-bbbbbbb";
        let releases = [Release {
            prerelease: true,
            nightly: Some(tag.into()),
            ..release(tag, "2026-11-22", false)
        }];
        let installed = [pruned.to_owned()];
        assert!(lines(&releases, &installed, None, Channel::Stable, "").is_empty());
        assert_eq!(
            shape(&lines(&releases, &installed, None, Channel::Dev, "")),
            ["# Installed", pruned, "# Dev · November 2026", tag]
        );
    }

    #[test]
    fn installed_builds_lead_then_the_rest_by_channel_and_month() {
        let releases = [
            release("nightly-2026-10-30-ccccccc", "2026-10-30", true),
            release("v2026.10.13", "2026-10-13", false),
            release("26.3-0.1.0", "2026-10-01", false),
        ];
        let installed = [
            "nightly-2026-11-17-aaaaaaa".to_owned(),
            "nightly-2026-11-16-bbbbbbb".to_owned(),
        ];
        let all = lines(
            &releases,
            &installed,
            Some("nightly-2026-11-16-bbbbbbb"),
            Channel::Nightly,
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
                "26.3-0.1.0",
            ]
        );
        // A commit or a day finds its build; a heading with nothing under it is gone.
        let found = lines(&releases, &installed, None, Channel::Nightly, "ccccccc");
        assert_eq!(
            shape(&found),
            ["# Nightly · October 2026", "nightly-2026-10-30-ccccccc"]
        );
        assert_eq!(
            shape(&lines(
                &releases,
                &installed,
                None,
                Channel::Nightly,
                "nov 17"
            ))
            .len(),
            2
        );
    }

    #[test]
    fn fit_preview_refreshes_on_new_index_serial_exact_age_and_due_timer() {
        let home = PathBuf::from("/tmp/profile");
        let completed = Instant::now();
        let checked = Arc::new(OnceLock::new());
        let _ = checked.set(Some(Vec::new()));
        let mut preview = FitPreview {
            home: home.clone(),
            tag: "nightly-2026-11-20-aaaaaaa".into(),
            inventory: 1,
            index: Some(10),
            completed: Some(completed),
            due: false,
            checked,
        };
        assert!(!preview.needs_refresh(
            &home,
            "nightly-2026-11-20-aaaaaaa",
            1,
            Some(10),
            completed,
        ));
        assert!(
            preview.needs_refresh(&home, "nightly-2026-11-20-aaaaaaa", 1, Some(11), completed,),
            "a newer signed index serial must invalidate the picker preview"
        );
        assert!(!preview.needs_refresh(
            &home,
            "nightly-2026-11-20-aaaaaaa",
            1,
            Some(10),
            completed + PREVIEW_RECHECK - std::time::Duration::from_millis(1),
        ));
        assert!(preview.needs_refresh(
            &home,
            "nightly-2026-11-20-aaaaaaa",
            1,
            Some(10),
            completed + PREVIEW_RECHECK,
        ));
        preview.due = true;
        assert!(preview.needs_refresh(&home, "nightly-2026-11-20-aaaaaaa", 1, Some(10), completed,));
    }

    #[test]
    fn fit_preview_timer_wakes_and_retries_a_failed_check_without_extra_input() {
        const CHILD: &str = "SUNRISE_PREVIEW_RETRY_TEST";
        if std::env::var_os(CHILD).is_none() {
            let scratch = tempfile::tempdir().unwrap();
            let root = scratch.path();
            let nowhere = "http://127.0.0.1:9";
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "sheets::versions::tests::fit_preview_timer_wakes_and_retries_a_failed_check_without_extra_input",
                ])
                .env(CHILD, "1")
                .env("RUST_MIN_STACK", (16 * 1024 * 1024).to_string())
                .env("SUNRISE_LAUNCHER_HOME", root.join("launcher"))
                .env("SUNRISE_HOME", root.join("game"))
                .env("SUNRISE_LAUNCHER_LOCATION_FILE", root.join("location.json"))
                .env("SUNRISE_MOD_INDEX_V2", root.join("missing-index.signed"))
                .env("SUNRISE_NO_KEYCHAIN", "1")
                .env("SUNRISE_TELEMETRY_URL", "off")
                .env("SUNRISE_LAUNCHER_NO_SELF_UPDATE", "1")
                .env("XDG_DATA_HOME", root.join("data"))
                .env("XDG_CONFIG_HOME", root.join("config"))
                .env("HTTPS_PROXY", nowhere)
                .env("HTTP_PROXY", nowhere)
                .env("ALL_PROXY", nowhere)
                .env_remove("NO_PROXY")
                .env_remove("SUNRISE_GITHUB_MIRROR")
                .env_remove("SUNRISE_LAUNCHER_BENCH")
                .status()
                .unwrap();
            assert!(status.success());
            return;
        }
        crate::wake::UNDER_TEST_SCHEDULER.store(true, std::sync::atomic::Ordering::Relaxed);
        let mut app = gpui::TestAppContext::single();
        let (shell, cx) = app.add_window_view(crate::window_root::Shell::new_for_test);
        let launcher = shell.read_with(cx, |shell, _| shell.launcher());
        cx.run_until_parked();
        const CURRENT: &str = "nightly-2026-11-22-bbbbbbb";
        const PICK: &str = "nightly-2026-11-20-aaaaaaa";
        launcher.update(cx, |l, cx| {
            l.releases = Load::Ready(vec![
                release(CURRENT, "2026-11-22", true),
                release(PICK, "2026-11-20", true),
            ]);
            l.open_versions();
            l.ui.sheets.versions.pick = Some(PICK.into());
            cx.notify();
        });
        cx.run_until_parked();
        let first = launcher
            .read_with(cx, |l, _| {
                l.ui.sheets
                    .versions
                    .preview
                    .as_ref()
                    .map(|p| p.checked.clone())
            })
            .expect("first preview check started");
        assert!(
            matches!(first.get(), Some(None)),
            "offline preview check should finish with None"
        );
        cx.executor().advance_clock(PREVIEW_RETRY);
        cx.run_until_parked();
        let second = launcher
            .read_with(cx, |l, _| {
                l.ui.sheets
                    .versions
                    .preview
                    .as_ref()
                    .map(|p| p.checked.clone())
            })
            .expect("retry preview check started");
        assert!(
            !Arc::ptr_eq(&first, &second),
            "scheduled retry timer must start a fresh preview check"
        );
    }
}
