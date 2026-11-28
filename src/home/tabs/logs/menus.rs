//! The Logs tab's search row, drawn as the other tabs' is: the source (and under a list, the file)
//! as compact dropdowns, the search field, then the level, Follow and a "…" menu on the right.
use super::sources::{file_label, pick_index};
use super::*;
use crate::widgets::button::{icon_button_opening, icon_only};
use crate::widgets::dropdown::{self, row_when, trigger, with_keys, Close, Face};
use crate::widgets::focus::c;
use gpui::{AnyElement, App, ElementId};
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// The most files of a list shown as choices; older ones stay in the folder.
const MAX_FILES: usize = 20;

/// A menu's least width: room for "Crash · 3 days ago" and the time under it.
const MENU_MIN: f32 = 200.;

/// Which of the row's menus is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Menu {
    Source,
    File,
    Level,
    More,
}

/// One row of a picker.
struct Choice {
    label: SharedString,
    sub: Option<SharedString>,
    current: bool,
    enabled: bool,
}

impl Choice {
    fn new(label: impl Into<SharedString>, current: bool) -> Self {
        Self {
            label: label.into(),
            sub: None,
            current,
            enabled: true,
        }
    }
}

/// What a row of the "…" menu does.
#[derive(Clone)]
enum More {
    CopyAll,
    Share,
    OpenFile(PathBuf),
    OpenFolder(PathBuf),
}

/// Toggling and closing the menu `which`: opening it closes whatever else is open.
fn menu_handles(cx: &mut Context<Launcher>, which: Menu) -> (Close, Close) {
    let (toggler, closer) = (cx.entity(), cx.entity());
    let toggle: Close = Rc::new(move |_: &mut Window, cx: &mut App| {
        toggler.update(cx, |this, cx| {
            let was = this.ui.home.tabs.logs.menu;
            this.close_menus();
            this.ui.home.tabs.logs.menu = (was != Some(which)).then_some(which);
            cx.notify();
        })
    });
    let close: Close = Rc::new(move |_: &mut Window, cx: &mut App| {
        closer.update(cx, |this, cx| {
            this.ui.home.tabs.logs.menu = None;
            cx.notify();
        })
    });
    (toggle, close)
}

impl Launcher {
    /// A dropdown field as tall as the search field beside it, listing `choices`; `pick` runs with
    /// the index of the one chosen.
    fn picker(
        &self,
        cx: &mut Context<Self>,
        id: &'static str,
        which: Menu,
        glyph: Option<&'static str>,
        choices: Vec<Choice>,
        pick: impl Fn(&mut Launcher, usize) + 'static,
    ) -> Div {
        let key = SharedString::from(id);
        let open = self.ui.home.tabs.logs.menu == Some(which);
        let text = choices
            .iter()
            .find(|choice| choice.current)
            .or(choices.first())
            .map(|choice| choice.label.clone())
            .unwrap_or_else(|| SharedString::from(""));
        let face = Face {
            text,
            sizers: choices.iter().map(|choice| choice.label.clone()).collect(),
            glyph,
            aside: None,
        };
        let (toggle, close) = menu_handles(cx, which);
        let press = toggle.clone();
        let field = trigger(key.clone(), open, face, true, false)
            .h(px(tokens::CONTROL))
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(move |_, window, cx| press(window, cx));
        let field = with_keys(field, open, toggle, close.clone());
        let wrap = div().relative().flex().flex_none().flex_col().child(field);
        if !open {
            dropdown::forget(&key);
            return wrap;
        }
        let (pick, entity) = (Rc::new(pick), cx.entity());
        let current = choices.iter().position(|choice| choice.current);
        let count = choices.len();
        let rows: Vec<AnyElement> = choices
            .into_iter()
            .enumerate()
            .map(|(n, choice)| {
                let line = row_when(
                    ElementId::from((SharedString::from(format!("{id}-row")), n)),
                    choice.label,
                    choice.sub,
                    choice.current,
                    choice.enabled,
                );
                if !choice.enabled {
                    return line.into_any_element();
                }
                let (pick, entity, close) = (pick.clone(), entity.clone(), close.clone());
                line.on_click(move |_, window, cx| {
                    entity.update(cx, |this, cx| {
                        pick(this, n);
                        cx.notify();
                    });
                    close(window, cx);
                })
                .into_any_element()
            })
            .collect();
        wrap.children(dropdown::layer(
            &key,
            rows,
            count,
            current,
            Some(MENU_MIN),
            close,
        ))
    }

    /// Which log: the game's output, `latest.log`, crash reports, the launcher's or the build's.
    fn source_picker(&self, cx: &mut Context<Self>) -> Div {
        let current = self.ui.home.tabs.logs.source;
        let choices = Source::ALL
            .iter()
            .map(|source| Choice::new(source.label(), *source == current))
            .collect();
        self.picker(
            cx,
            "home-logs-source",
            Menu::Source,
            Some("computer-terminal-01"),
            choices,
            |this, n| {
                if let Some(source) = Source::ALL.get(n) {
                    this.ui.home.tabs.logs.switch(*source);
                }
            },
        )
    }

    /// Under a list source, which of its files, newest first; none for the other sources or an
    /// empty folder.
    fn file_picker(&self, cx: &mut Context<Self>) -> Option<Div> {
        let logs = &self.ui.home.tabs.logs;
        if !logs.source.is_list() || logs.files.is_empty() {
            return None;
        }
        let current = pick_index(&logs.files, logs.chosen.as_deref()).unwrap_or(0);
        let mut choices: Vec<Choice> = logs
            .files
            .iter()
            .take(MAX_FILES)
            .enumerate()
            .map(|(n, path)| {
                let time = logs.times.get(n).copied().flatten();
                file_choice(logs.source, n, path, time, n == current)
            })
            .collect();
        let older = logs.files.len().saturating_sub(MAX_FILES);
        if older > 0 {
            let mut note = Choice::new(
                trf(
                    "{n} older ones are in the folder",
                    &[("n", &older.to_string())],
                ),
                false,
            );
            note.enabled = false;
            choices.push(note);
        }
        Some(self.picker(
            cx,
            "home-logs-file",
            Menu::File,
            None,
            choices,
            |this, n| {
                let logs = &mut this.ui.home.tabs.logs;
                if let Some(path) = logs.files.get(n) {
                    logs.chosen = Some(path.clone());
                    logs.seen.set((u64::MAX, 0));
                }
            },
        ))
    }

    /// All, Warnings or Errors: which lines the log shows.
    fn level_picker(&self, cx: &mut Context<Self>) -> Div {
        let current = self.ui.home.tabs.logs.level;
        let choices = Level::ALL
            .iter()
            .map(|level| Choice::new(level.label(), *level == current))
            .collect();
        self.picker(
            cx,
            "home-logs-level",
            Menu::Level,
            Some("sliders-horizontal"),
            choices,
            |this, n| {
                if let Some(level) = Level::ALL.get(n) {
                    let logs = &mut this.ui.home.tabs.logs;
                    logs.level = *level;
                    logs.seen.set((u64::MAX, 0));
                }
            },
        )
    }

    /// Follow, an icon beside the menu: lit while the view keeps the newest line in sight.
    fn follow_button(&self, cx: &mut Context<Self>) -> Stateful<Div> {
        let style = match self.ui.home.tabs.logs.follow {
            true => Style::Accent,
            false => Style::Secondary,
        };
        icon_only(
            "home-logs-follow",
            Look::new(style, Size::Medium).icon("arrow-down-01"),
        )
        .tooltip(crate::glass_ui::tip(tr("Follow new lines")))
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
            let logs = &mut this.ui.home.tabs.logs;
            logs.follow = !logs.follow;
            // Switching it on goes to the newest line at once.
            logs.seen.set((u64::MAX, 0));
            cx.notify();
        }))
    }

    /// The "…" menu: Copy all, Share log, Open file, Open logs folder. It hangs from the button's
    /// right edge, so it stays inside the window.
    fn more_menu(&self, home: &Path, folder: &Path, cx: &mut Context<Self>) -> Div {
        let logs = &self.ui.home.tabs.logs;
        let open = logs.menu == Some(Menu::More);
        let (toggle, close) = menu_handles(cx, Menu::More);
        let press = toggle.clone();
        let button = icon_button_opening(
            "home-logs-more",
            "more-horizontal",
            tr("More actions"),
            Style::Ghost,
            open,
        )
        .on_click(move |_, window, cx| press(window, cx));
        let button = with_keys(button, open, toggle, close.clone());
        let wrap = div().relative().flex_none().child(button);
        if !open {
            return wrap;
        }
        let mut rows = vec![(tr("Copy all"), true, More::CopyAll)];
        rows.push((tr("Share log…"), !logs.sharing, More::Share));
        if let Some(file) = logs.target(home).filter(|path| path.is_file()) {
            rows.push((tr("Open file"), true, More::OpenFile(file)));
        }
        rows.push((
            tr("Open logs folder"),
            true,
            More::OpenFolder(folder.to_path_buf()),
        ));
        let (entity, home) = (cx.entity(), home.to_path_buf());
        let rows = rows
            .into_iter()
            .enumerate()
            .map(|(n, (label, enabled, action))| {
                let line = row_when(
                    ElementId::from((SharedString::from("home-logs-more-row"), n)),
                    label,
                    None,
                    false,
                    enabled,
                );
                if !enabled {
                    return line.into_any_element();
                }
                let (entity, close, home) = (entity.clone(), close.clone(), home.clone());
                line.on_click(move |_, window, cx| {
                    close(window, cx);
                    entity.update(cx, |this, cx| {
                        this.run_more(&home, &action, cx);
                        cx.notify();
                    });
                })
                .into_any_element()
            });
        wrap.children(more_layer(rows.collect(), close))
    }

    /// What a row of the "…" menu does for the profile at `home`.
    fn run_more(&mut self, home: &Path, action: &More, cx: &mut Context<Self>) {
        match action {
            More::CopyAll => {
                let text = self.copy_text(home);
                crate::selectable::clipboard::copy_to_clipboard(
                    cx,
                    gpui::ClipboardItem::new_string(text),
                );
            }
            More::Share => self.ui.home.tabs.logs.confirm = true,
            More::OpenFile(path) => crate::toast::open(cx, path),
            More::OpenFolder(folder) => open_logs(cx, folder),
        }
    }

    /// The search field: the text lines must contain, Escape to leave it.
    fn log_search(
        &self,
        focus: &FocusHandle,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let t = crate::mods::field_theme(self.theme);
        let focused = focus.is_focused(window);
        let field = crate::widgets::field::search_box(
            focused,
            None,
            line_edit::content(
                &self.ui.home.tabs.logs.search,
                &t,
                focused,
                false,
                false,
                "Search the log",
            ),
        )
        .id("home-logs-search")
        .track_focus(focus)
        .seen()
        .flex_1()
        .min_w(px(120.))
        .max_w(px(260.))
        .h(px(tokens::CONTROL))
        .cursor_text()
        .on_click({
            let focus = focus.clone();
            cx.listener(move |_, _: &ClickEvent, window, cx| {
                window.focus(&focus, cx);
                cx.notify();
            })
        })
        .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
            match line_edit::handle(&mut this.ui.home.tabs.logs.search, ev, cx) {
                Key::Escape => window.blur(cx),
                Key::Pass => return,
                _ => {}
            }
            cx.stop_propagation();
            cx.notify();
        }));
        line_edit::mouse(field, cx, |this: &mut Launcher| {
            &mut this.ui.home.tabs.logs.search
        })
    }

    /// The row under the tabs, in the height and gaps of the other tabs' search row: the pickers
    /// first; with lines to show, the search, and on the right the level, Follow and the menu.
    pub(super) fn log_toolbar(
        &self,
        focus: &FocusHandle,
        home: &Path,
        folder: &Path,
        has_lines: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let sources = self.source_picker(cx);
        let files = self.file_picker(cx);
        let search = has_lines.then(|| self.log_search(focus, window, cx));
        let level = has_lines.then(|| self.level_picker(cx));
        let follow = has_lines.then(|| self.follow_button(cx));
        let more = has_lines.then(|| self.more_menu(home, folder, cx));
        div()
            .flex()
            .flex_none()
            // 15 px under the tabs and 12 before the log, around the 32 px controls, as on every
            // tab that has a search.
            .pt(px(15.))
            .pb(px(12.))
            .h(px(59.))
            .items_center()
            .gap(px(8.))
            .child(sources)
            .children(files)
            .children(search)
            .child(div().flex_1().min_w_0())
            .children(level)
            .children(follow)
            .children(more)
    }
}

/// A row of a crash report or launcher failure: when it happened, in words, with the date and
/// time under it. Without a time it is its name, or its place in the list.
fn file_choice(
    source: Source,
    index: usize,
    path: &Path,
    time: Option<std::time::SystemTime>,
    current: bool,
) -> Choice {
    let Some(time) = time else {
        return Choice::new(file_label(source, index, path), current);
    };
    let when = crate::worlds_page::ago(time);
    let label = match source {
        Source::Crashes => trf("Crash · {when}", &[("when", &when)]),
        _ => trf("Launch failure · {when}", &[("when", &when)]),
    };
    let at: chrono::DateTime<chrono::Local> = time.into();
    let mut choice = Choice::new(label, current);
    choice.sub = Some(at.format("%Y-%m-%d %H:%M").to_string().into());
    choice
}

/// The open "…" menu over everything after it, with a backdrop whose press closes it: the
/// dropdown's look, anchored to the right of its button.
fn more_layer(rows: Vec<AnyElement>, close: Close) -> [gpui::Deferred; 2] {
    use gpui::AnimationExt as _;
    let p = palette();
    let backdrop = {
        let close = close.clone();
        div()
            .absolute()
            .top(px(-4000.))
            .left(px(-4000.))
            .w(px(8000.))
            .h(px(8000.))
            .occlude()
            .on_mouse_down(gpui::MouseButton::Left, move |_, window, cx| {
                close(window, cx)
            })
    };
    let below = tokens::CONTROL + dropdown::GAP;
    let menu = div()
        .id("home-logs-more-menu")
        .absolute()
        .right_0()
        .w(px(MENU_MIN))
        .flex()
        .flex_col()
        .gap(px(1.))
        .p(px(4.))
        .rounded(tokens::r(tokens::R_CONTROL))
        .border_1()
        .border_color(p.input_edge())
        .bg(c(p.raised))
        .shadow_lg()
        .occlude()
        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .children(rows);
    let menu = dropdown::with_keys(menu, true, close.clone(), close).with_animation(
        "home-logs-more-menu-in",
        crate::motion::eased(sunrise_launcher_theme::QUICK),
        move |d, t| d.opacity(t).top(px(below - dropdown::GAP * (1. - t))),
    );
    [
        gpui::deferred(backdrop).with_priority(9),
        gpui::deferred(menu).with_priority(10),
    ]
}
