//! Settings for the whole launcher, one section at a time. The game build and its channel stay
//! in the play bar, the one place that shows them.
use super::*;
use crate::frame::button;
use crate::frame::Reach as _;
use crate::mods::{choices, switch, Choose};
use crate::paint::palette;
use gpui::rgb;
use std::rc::Rc;
use storage::Category;
use sunrise_launcher::telemetry;
use sunrise_launcher_telemetry::Choices;

mod appearance;
pub(crate) use appearance::HomePhoto;

#[derive(Clone, Copy, Default, Eq, PartialEq)]
pub(super) enum Section {
    #[default]
    Account,
    Game,
    Appearance,
    Storage,
    Privacy,
    Launcher,
}

/// The section list's width, the room beside it and the widest a section gets: together, the
/// block Settings centres on a wide window.
const NAV_WIDTH: f32 = 180.;
const COLUMN_GAP: f32 = 28.;
const PANE_WIDTH: f32 = 760.;
const COLUMNS: f32 = NAV_WIDTH + COLUMN_GAP + PANE_WIDTH;

/// The Settings section the window opens on, when what the last start left needs one: a data
/// folder that did not open or a move that failed or finished (Storage has its recoveries).
pub(super) fn startup_section(location_error: bool, location_note: bool) -> Option<Section> {
    (location_error || location_note).then_some(Section::Storage)
}

impl Section {
    pub(super) const ALL: [Section; 6] = [
        Section::Account,
        Section::Game,
        Section::Appearance,
        Section::Storage,
        Section::Privacy,
        Section::Launcher,
    ];

    pub(super) fn label(self) -> &'static str {
        match self {
            Section::Account => "Account",
            Section::Game => "Game",
            Section::Appearance => "Appearance",
            Section::Storage => "Storage",
            Section::Privacy => "Privacy",
            Section::Launcher => "Launcher",
        }
    }

    /// Its icon in the list, from the glass set the top tabs draw theirs from.
    fn icon(self) -> &'static str {
        match self {
            Section::Account => "user",
            Section::Game => "game-controller-01",
            Section::Appearance => "color-picker",
            Section::Storage => "folder-01",
            Section::Privacy => "shield",
            // Language and network: no launcher or app icon in the set.
            Section::Launcher => "globe-02",
        }
    }

    fn about(self) -> &'static str {
        match self {
            Section::Account => "Who plays, and the Microsoft accounts signed in here.",
            Section::Game => "How the game starts. Its build is chosen beside Play on Home.",
            Section::Appearance => "The theme, its accent and what is behind Home.",
            Section::Storage => "Manage game builds, downloads and where your files are kept.",
            Section::Privacy => "What Sunrise sends, and what it never does.",
            Section::Launcher => "Language, network and this version.",
        }
    }
}

#[derive(Default)]
pub(super) struct SettingsView {
    pub(super) section: Section,
    languages: bool,
    /// UUID of the account whose 3-dots menu is open in Settings › Account.
    account_menu: Option<String>,
    setup: bool,
    note: Option<String>,
    pub(crate) appearance: appearance::State,
    /// The Storage page's count or Clear in flight (`glass_storage`).
    pub(super) storage_work: Option<glass_storage::Work>,
    /// Feedback from the last explicit Clear, independent of the next count.
    pub(super) storage_clear: Option<Result<Box<storage::Report>, String>>,
}

impl SettingsView {
    pub(super) fn storage_busy(&self) -> bool {
        self.storage_work.is_some()
    }
}

/// A card of rows with a rule between them.
pub(crate) fn group(rows: Vec<Div>) -> Div {
    let mut card = div()
        .flex()
        .flex_col()
        .rounded(crate::tokens::r(crate::tokens::R_CARD))
        .border_1()
        .border_color(rgb(palette().divider))
        .bg(palette().card());
    for (i, row) in rows.into_iter().enumerate() {
        card = card.child(
            row.id(("settings-row", i))
                .px(px(16.))
                .py(px(14.))
                .when(i > 0, |d| {
                    d.border_t_1().border_color(rgb(palette().divider))
                }),
        );
    }
    card
}

pub(crate) fn hint(text: impl Into<SharedString>) -> Div {
    div()
        .text_color(rgb(palette().muted))
        .text_size(px(tokens::SMALL))
        .child(crate::selectable::selectable(text))
}

pub(crate) fn row(
    title: impl Into<SharedString>,
    about: impl Into<SharedString>,
    actions: Div,
) -> Div {
    let about = about.into();
    // When the words would get narrower than a short line, the controls go under them instead:
    // the words never squeeze to a word a line beside them.
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap_x(px(16.))
        .gap_y(px(10.))
        .child(
            div()
                .flex_1()
                .min_w(px(160.))
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(crate::selectable::selectable(title)),
                )
                .when(!about.is_empty(), |d| d.child(hint(about))),
        )
        .child(actions.flex_none().flex().items_center().gap(px(6.)))
}

/// A row whose control needs the full width: the words above, the control below.
fn stacked(title: impl Into<SharedString>, about: impl Into<SharedString>, below: Div) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(10.))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(crate::selectable::selectable(title)),
                )
                .child(hint(about)),
        )
        .child(below)
}

/// How many characters a settings text box shows at this window width. The pane is as wide as
/// the window less the sidebar, the categories and the margins (a little over 500 px, so a
/// guess here errs short), up to 760 px; the box takes 26 px of padding and edge, and a
/// character of its 13 px mono is about 8 px.
fn field_chars(window_width: f32) -> usize {
    let pane = (window_width - 520.).clamp(200., 760.);
    ((pane - 26.) / 8.) as usize
}

/// Sizes a settings text box to its pane: the one place the box's width is set.
fn fit_field(input: &mut line_edit::LineEdit, window_width: f32) {
    input.show(field_chars(window_width));
}

/// Writes a failed launcher update check's full error (it holds URLs and local addresses) to
/// the launcher's log through `log`; the window shows [`launcher_check_line`]'s plain line.
pub(crate) fn log_launcher_check_failure(error: &str, log: impl FnOnce(&str)) {
    log(&format!("Launcher update check failed: {error}"));
}

/// What the launcher's own update row says when no update is found: a plain line for a failure
/// (a retry is offered beside it, and the detail is in the log), "checked just now" for a
/// minute after an explicit check, or just "up to date" once any check has run.
fn launcher_check_line(
    error: Option<&str>,
    checked_now: Option<Instant>,
    checked_before: bool,
) -> (String, bool) {
    match error {
        Some(_) => (
            tr("Couldn't check for updates. Check your connection and try again.").to_string(),
            true,
        ),
        None if checked_now.is_some_and(|at| at.elapsed() < Duration::from_secs(60)) => {
            (tr("up to date · checked just now").to_string(), false)
        }
        None if checked_before => (tr("up to date").to_string(), false),
        None => (String::new(), false),
    }
}

impl Launcher {
    /// A [`row`] whose controls are wide: in a narrow window they would leave the words a
    /// column a syllable wide, so there they go under the words.
    fn wide_row(
        &self,
        title: impl Into<SharedString>,
        about: impl Into<SharedString>,
        actions: Div,
    ) -> Div {
        if self.window_width < 960. {
            stacked(title, about, actions.flex().flex_wrap().gap(px(6.)))
        } else {
            row(title, about, actions)
        }
    }
}

/// For a box that scrolls inside the page: the wheel over it scrolls only the box. The box has
/// scrolled by the time this runs (an element's own scrolling hears the wheel first).
fn own_wheel(_: &gpui::ScrollEvent, _: &mut Window, cx: &mut App) {
    cx.stop_propagation();
}

pub(crate) fn off(b: Stateful<Div>) -> Stateful<Div> {
    b.opacity(0.4).cursor_default().tab_stop(false)
}

pub(crate) fn danger(b: Stateful<Div>) -> Stateful<Div> {
    b.text_color(rgb(palette().danger))
}

pub(crate) fn warning(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(tokens::SMALL))
        .text_color(rgb(palette().danger))
        .child(crate::selectable::selectable(text))
}

fn corner_px(value: f32) -> u8 {
    (value * f32::from(backend::MAX_CORNER_RADIUS)).round() as u8
}

impl Launcher {
    pub(super) fn settings(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        self.args_focused = self.args_focus.is_focused(window);
        self.proxy_focused = self.proxy_focus.is_focused(window);
        self.corners_focused = self.corners_focus.is_focused(window);
        let section = self.ui.settings.section;
        let scroll = crate::frame::scrolls("settings");
        let mut nav = div()
            .relative()
            .top(px(self.settings_sticky.shift(&scroll, 0, 0)))
            .flex()
            .flex_col()
            .gap(px(2.))
            .w(px(NAV_WIDTH))
            .flex_none();
        for (n, s) in Section::ALL.into_iter().enumerate() {
            let on = s == section;
            let colour = if on { palette().text } else { palette().muted };
            nav = nav.child(
                div()
                    .id(ElementId::named_usize("alt-settings-nav", n))
                    .group("alt-settings-nav")
                    .reach()
                    .h(px(tokens::FIELD))
                    .px(px(12.))
                    .flex()
                    .items_center()
                    .gap(px(7.))
                    .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
                    .cursor_pointer()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(colour))
                    .when(on, |d| d.bg(rgb(palette().selected)))
                    .hover(|d| d.text_color(rgb(palette().text)))
                    // The top tabs' icon size and gap; it lights up with its label.
                    .child(
                        crate::glass_ui::icon(s.icon(), 16., rgb(colour).into())
                            .group_hover("alt-settings-nav", |d| d.text_color(rgb(palette().text))),
                    )
                    .child(tr(s.label()))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.settings_open(s);
                        cx.notify();
                    })),
            );
        }
        let body = match section {
            Section::Account => self.settings_account(cx),
            Section::Game => self.settings_game(cx),
            Section::Appearance => self.settings_appearance(window, cx),
            Section::Storage => self.settings_storage(cx),
            Section::Privacy => self.settings_privacy(cx),
            Section::Launcher => self.settings_launcher(cx),
        };
        let pane = div()
            .flex_1()
            .min_w_0()
            .max_w(px(PANE_WIDTH))
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        div()
                            .text_size(px(tokens::HEADING))
                            .font_weight(FontWeight::BOLD)
                            .child(crate::selectable::selectable(tr(section.label()))),
                    )
                    .child(hint(tr(section.about()))),
            )
            .children(self.location_error.clone().map(warning))
            .child(body);
        // The title stays put above the scroll, and the categories stay in view as the section
        // scrolls (`Sticky`, which reads this row as the scroll's item 0).
        div()
            .size_full()
            .flex()
            .flex_col()
            .text_color(rgb(palette().text))
            .child(
                // Over the section list, wherever the block sits.
                div()
                    .w_full()
                    .max_w(px(COLUMNS + 2. * 32.))
                    .mx_auto()
                    .px(px(32.))
                    .pt(px(24.))
                    .pb(px(20.))
                    .text_size(px(tokens::DISPLAY))
                    .font_weight(FontWeight::BOLD)
                    .child(tr("Settings")),
            )
            .child(crate::scrollbar::wrap_page(
                "settings",
                div()
                    .id("alt-settings-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(px(32.))
                    .pb(px(24.))
                    .child(
                        div()
                            .id("alt-settings-columns")
                            .track_scroll(self.settings_sticky.row())
                            // On a wide window the block keeps its width, in the middle.
                            .w_full()
                            .max_w(px(COLUMNS))
                            .mx_auto()
                            .flex()
                            .items_start()
                            .gap(px(COLUMN_GAP))
                            .child(nav)
                            .child(pane),
                    ),
                &self.theme,
            ))
            // A press anywhere else closes open dropdown menus; the menus and their buttons keep
            // their own presses.
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    let closed_lang = std::mem::take(&mut this.ui.settings.languages);
                    let closed_account = this.ui.settings.account_menu.take().is_some();
                    if closed_lang || closed_account {
                        cx.notify();
                    }
                }),
            )
            .into_any_element()
    }

    /// Opens a section, leaving no question from another one standing.
    pub(super) fn settings_open(&mut self, section: Section) {
        // Another section starts at its top, with its list of sections back in place: the old
        // section's scroll would cut the new one's title off, and the list would hold on to
        // where the old one had pushed it.
        let changed = self.ui.settings.section != section;
        if changed {
            crate::frame::scrolls("settings").set_offset(gpui::point(px(0.), px(0.)));
            self.settings_sticky.reset();
        }
        let view = &mut self.ui.settings;
        view.section = section;
        view.languages = false;
        view.account_menu = None;
        view.setup = false;
        view.note = None;
        self.storage_confirm = None;
        self.storage_builds.confirm = None;
        self.collection.confirm_delete = false;
        if section == Section::Storage {
            if changed {
                self.refresh_storage(None, false);
                self.refresh_open_storage_builds();
            }
        } else {
            self.stop_storage_work();
        }
    }

    fn remove_account(&mut self, uuid: &str) {
        if self.account_locked() {
            self.say_account_locked();
            return;
        }
        let Some(generation) = self.begin_auth_job(uuid) else {
            return;
        };
        let (tx, uuid) = (self.tx.clone(), uuid.to_owned());
        std::thread::spawn(move || {
            let mut accounts = Accounts::load();
            let _ = tx.send(Msg::AccountProgress(
                uuid.clone(),
                generation,
                account::session::Phase::SigningOut,
            ));
            let result = accounts
                .remove(account::store(), &uuid)
                .and_then(|()| accounts.save())
                .map(|()| accounts.has_owner(account::store()));
            let _ = tx.send(Msg::AccountRemoved(uuid, generation, result));
        });
    }

    fn move_account(&mut self, uuid: &str, delta: isize) {
        match Accounts::reorder(uuid, delta) {
            Ok(accounts) => self.accounts = accounts,
            Err(error) => self.ui.settings.note = Some(error),
        }
    }

    fn settings_account(&mut self, cx: &mut Context<Self>) -> Div {
        let theme = self.theme;
        let selected = self.accounts.selected.clone();
        let total = self.accounts.accounts.len();
        let open_menu = self.ui.settings.account_menu.clone();
        let mut rows = Vec::new();
        for (i, a) in self.accounts.accounts.clone().into_iter().enumerate() {
            let plays = selected.as_ref() == Some(&a.uuid);
            let status = self.auth_status.get(&a.uuid);
            let needs_sign_in =
                status.is_some_and(|s| s.state == account::session::State::SignInRequired);
            let progress = self.auth_progress.get(&a.uuid).map(|phase| phase.label());
            let busy = self.auth_busy.contains_key(&a.uuid);
            let said = match (progress, status) {
                (Some(doing), _) => tr(doing).to_string(),
                (None, Some(s)) if needs_sign_in => tr(&s.message).to_string(),
                _ if plays => tr("Plays when you press Play").to_string(),
                _ => tr("Signed in").to_string(),
            };
            let mut actions = div();
            if needs_sign_in {
                let uuid = a.uuid.clone();
                let again = button(
                    ElementId::named_usize("alt-account-again", i),
                    tr("Sign in again"),
                );
                actions = actions.child(if busy {
                    off(again)
                } else {
                    again.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.re_sign_in(&uuid);
                        cx.notify();
                    }))
                });
            } else {
                if !plays {
                    let uuid = a.uuid.clone();
                    actions = actions.child(
                        button(ElementId::named_usize("alt-account-play", i), tr("Play as"))
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.play_as(&uuid);
                                cx.notify();
                            })),
                    );
                }
                let uuid = a.uuid.clone();
                let out = button(ElementId::named_usize("alt-account-out", i), tr("Sign out"));
                actions = actions.child(if busy {
                    off(out)
                } else {
                    out.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.remove_account(&uuid);
                        cx.notify();
                    }))
                });
            }
            let menu_open = open_menu.as_deref() == Some(a.uuid.as_str());
            let toggle_uuid = a.uuid.clone();
            let dots = div()
                .id(ElementId::named_usize("alt-account-menu", i))
                .reach()
                .size(px(tokens::CONTROL))
                .rounded(tokens::r(tokens::R_CONTROL))
                .when(menu_open, |d| d.bg(rgb(palette().hover)))
                .hover(|s| s.bg(rgb(palette().hover)))
                .cursor_pointer()
                .flex()
                .items_center()
                .justify_center()
                .tooltip(crate::glass_ui::tip(tr("Account options")))
                .child(crate::glass_ui::icon(
                    "more-vertical",
                    16.,
                    rgb(palette().muted).into(),
                ))
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    if this.ui.settings.account_menu.as_deref() == Some(toggle_uuid.as_str()) {
                        this.ui.settings.account_menu = None;
                    } else {
                        this.ui.settings.account_menu = Some(toggle_uuid.clone());
                    }
                    cx.notify();
                }));
            let menu = menu_open.then(|| {
                let item = |id: (&'static str, usize), label: &'static str, danger: bool| {
                    div()
                        .id(id)
                        .reach()
                        .flex()
                        .items_center()
                        .px(px(10.))
                        .py(px(7.))
                        .rounded(tokens::r(tokens::R_SMALL))
                        .text_size(px(tokens::LABEL))
                        .text_color(rgb(if danger {
                            palette().danger
                        } else {
                            palette().text
                        }))
                        .cursor_pointer()
                        .hover(|s| s.bg(rgb(palette().hover)))
                        .child(label)
                };
                let mut list = div()
                    .id(ElementId::named_usize("alt-account-menu-list", i))
                    .w(px(180.))
                    .p(px(6.))
                    .rounded(tokens::r(tokens::R_CONTROL))
                    .border_1()
                    .border_color(palette().input_edge())
                    .bg(rgb(palette().raised))
                    .shadow_lg()
                    .occlude()
                    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .flex()
                    .flex_col();
                if !plays {
                    let uuid = a.uuid.clone();
                    list = list.child(
                        item(("alt-account-default", i), tr("Set default"), false).on_click(
                            cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.ui.settings.account_menu = None;
                                this.play_as(&uuid);
                                cx.notify();
                            }),
                        ),
                    );
                }
                if i > 0 {
                    let uuid = a.uuid.clone();
                    list = list.child(item(("alt-account-up", i), tr("Move up"), false).on_click(
                        cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.ui.settings.account_menu = None;
                            this.move_account(&uuid, -1);
                            cx.notify();
                        }),
                    ));
                }
                if i + 1 < total {
                    let uuid = a.uuid.clone();
                    list = list.child(
                        item(("alt-account-down", i), tr("Move down"), false).on_click(
                            cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.ui.settings.account_menu = None;
                                this.move_account(&uuid, 1);
                                cx.notify();
                            }),
                        ),
                    );
                }
                {
                    let uuid = a.uuid.clone();
                    list = list.child(
                        item(("alt-account-skins", i), tr("Manage skins"), false).on_click(
                            cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.ui.settings.account_menu = None;
                                if this.accounts.selected.as_deref() != Some(uuid.as_str()) {
                                    if this.account_locked() {
                                        this.say_account_locked();
                                        cx.notify();
                                        return;
                                    }
                                    this.play_as(&uuid);
                                }
                                this.navigate(crate::frame::Page::Skins);
                                cx.notify();
                            }),
                        ),
                    );
                }
                {
                    let uuid = a.uuid.clone();
                    let remove = item(("alt-account-remove", i), tr("Remove"), true);
                    list = list.child(if busy {
                        off(remove)
                    } else {
                        remove.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.ui.settings.account_menu = None;
                            this.remove_account(&uuid);
                            cx.notify();
                        }))
                    });
                }
                gpui::deferred(
                    div()
                        .absolute()
                        .top(px(tokens::CONTROL + 6.))
                        .right_0()
                        .child(list),
                )
                .with_priority(1)
            });
            actions = actions.child(div().relative().child(dots).children(menu));
            // As in `row`: when the head and name would get narrower than a short line, the
            // buttons go under them instead of squeezing the name to a letter a line.
            let who = div()
                .flex_1()
                .min_w(px(160. + 36. + 12.))
                .flex()
                .items_center()
                .gap(px(12.))
                .child(
                    div()
                        .flex_none()
                        .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
                        .overflow_hidden()
                        .child(
                            self.profile_heads
                                .draw(Some(&a.uuid), 36., self.scale, &theme),
                        ),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(
                            div()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(a.name.clone()),
                        )
                        .child(if needs_sign_in {
                            warning(said)
                        } else {
                            hint(said)
                        }),
                );
            rows.push(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_x(px(16.))
                    .gap_y(px(10.))
                    .child(who)
                    .child(actions.flex_none().flex().items_center().gap(px(6.))),
            );
        }
        let none = rows.is_empty();
        rows.push(row(
            tr(if none {
                "Not signed in"
            } else {
                "Another account"
            }),
            tr("Sign in with the Microsoft account that owns Minecraft: Java Edition."),
            div().child(
                button("alt-add-account", tr("Sign in with Microsoft")).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        this.sign_in_to_play();
                        cx.notify();
                    },
                )),
            ),
        ));
        group(rows)
    }

    fn settings_game(&mut self, cx: &mut Context<Self>) -> Div {
        use backend::WhilePlaying;
        let current = self.settings.while_playing.here();
        let mut ways = vec![
            (WhilePlaying::Stay, "Stay open"),
            (WhilePlaying::Minimize, "Minimise"),
        ];
        if cfg!(target_os = "macos") {
            ways.push((WhilePlaying::Hide, "Hide"));
        }
        let ways = ways
            .into_iter()
            .map(|(way, label)| {
                let pick: Choose = Rc::new(move |this, _| {
                    this.settings.while_playing = way;
                    this.save_settings();
                });
                (tr(label).into(), way == current, pick)
            })
            .collect();
        let picker = div().child(choices("alt-while-playing", ways, cx).gap(px(6.)));
        let while_playing = row(
            tr("While you play"),
            tr(match current {
                WhilePlaying::Stay => "The launcher stays open beside the game.",
                WhilePlaying::Minimize => {
                    "The launcher goes down when the game starts, and comes back when it closes."
                }
                WhilePlaying::Hide => {
                    "The launcher hides when the game starts, and comes back when it closes."
                }
            }),
            picker,
        );
        let minecraft = self.settings.minecraft_dir.as_deref().map_or_else(
            || {
                tr("Not found yet. Sunrise reads the game's own files from your Minecraft folder.")
                    .to_string()
            },
            short,
        );
        let source = row(
            tr("Minecraft folder"),
            minecraft,
            div().child(
                button("alt-minecraft-folder", tr("Choose…"))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.choose_folder(cx))),
            ),
        );
        let args = self.settings_field(
            "alt-game-args",
            self.args_focus.clone(),
            self.args_focused,
            &self.args_input,
            "For example --server-thread",
            |this| &mut this.args_input,
            |this| {
                this.settings.game_args = this.args_input.text().to_string();
                this.save_settings();
            },
            cx,
        );
        let args = stacked(
            tr("Game arguments"),
            tr("Extra arguments for the game, passed through as they are. They apply the next time it starts."),
            args,
        );
        let asking = self.ui.settings.setup;
        let mut setup = row(
            tr("Profiles"),
            tr("A profile keeps its own mods, worlds and servers. Switch between profiles in the sidebar."),
            div().child(
                button("alt-create", tr(if asking { "Cancel" } else { "New profile…" })).on_click(
                    cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.ui.settings.setup = !this.ui.settings.setup;
                        this.ui.settings.note = None;
                        cx.notify();
                    }),
                ),
            ),
        );
        if asking {
            let input = self.field(
                servers_page::Field::InstanceName,
                tr("Profile name"),
                250.,
                cx,
            );
            setup = div().flex().flex_col().gap(px(12.)).child(setup).child(
                div().flex().items_center().gap(px(8.)).child(input).child(
                    button("alt-create-empty", tr("Create profile")).on_click(cx.listener(
                        |this, _: &ClickEvent, _, cx| {
                            let name = this.servers.value(servers_page::Field::InstanceName).trim();
                            match instances::create(name, None) {
                                Ok(_) => {
                                    this.reload_instances();
                                    this.ui.settings.setup = false;
                                    this.servers
                                        .text(servers_page::Field::InstanceName)
                                        .set(String::new());
                                    this.ui.settings.note = Some(
                                        tr("Profile created. Select it in the sidebar.").into(),
                                    );
                                }
                                Err(e) => this.ui.settings.note = Some(e),
                            };
                            cx.notify();
                        },
                    )),
                ),
            );
        }
        if let Some(note) = self.ui.settings.note.clone() {
            setup = div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(setup)
                .child(hint(note));
        }
        let mut profiles = vec![setup];
        if let Some(profile) = self
            .selected_instance()
            .filter(|p| p.kind == instances::Kind::Own && !self.location_unavailable)
        {
            let target = profile.clone();
            profiles.push(row(
                trf("Selected profile: {name}", &[("name", &target.name)]),
                "",
                div().child(
                    danger(button("settings-delete-profile", tr("Delete profile…"))).on_click(
                        cx.listener(move |this, _, _, cx| {
                            this.confirm_profile_delete(&target);
                            cx.notify();
                        }),
                    ),
                ),
            ));
        }
        div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(group(vec![while_playing, args]))
            .child(group(vec![source]))
            .child(group(profiles))
    }

    fn settings_storage(&mut self, cx: &mut Context<Self>) -> Div {
        let game = backend::game_home();
        let files = row(
            tr("Game files and worlds"),
            short(&game),
            div().child({
                let dir = game.clone();
                button("alt-open-game", tr("Open folder"))
                    .on_click(move |_, _, cx| glass_toast::open(cx, &dir))
            }),
        );

        let busy = self.ui.settings.storage_busy();
        // The total waits for every row; a row shows its own size as soon as it is counted.
        let complete = self.storage_report.as_ref().filter(|r| r.is_complete());
        let total =
            complete.map(|report| report.rows.iter().map(|r| r.remaining_bytes).sum::<u64>());
        // Clear needs no size: a row not counted yet can be cleared like any other.
        let offered = glass_storage::clear_offered(
            self.ui.settings.storage_work.as_ref(),
            self.location_unavailable,
        );
        // Counting that failed is not still counting.
        let uncounted = if self.storage_error.is_some() && !busy {
            tr("Not counted")
        } else {
            tr("Counting…")
        };
        let clear_all = button("alt-storage-clear-all", tr("Clear all…"));
        let size = total.map_or_else(|| uncounted.to_string(), storage::format_bytes);
        let detail = trf(
            "{size}. Files Sunrise can fetch or make again; your worlds, mods and settings are never part of it.",
            &[("size", &size)],
        );
        let detail = if size.ends_with('…') {
            detail.replacen("….", "…", 1)
        } else {
            detail
        };
        let mut cache = vec![row(
            tr("Downloads kept for reuse"),
            detail,
            div().child(
                if offered
                    && complete.is_none_or(|r| {
                        r.rows
                            .iter()
                            .any(|row| glass_storage::row_clearable(Some(row)))
                    })
                {
                    clear_all.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.storage_confirm = Some(None);
                        cx.notify();
                    }))
                } else {
                    off(clear_all)
                },
            ),
        )];
        // Neither starting nor finishing an automatic count replaces Clear feedback.
        cache.extend(self.storage_error.as_ref().map(|error| hint(error.clone())));
        let cleared = match &self.ui.settings.storage_clear {
            Some(Ok(report)) => Some(report),
            Some(Err(error)) => {
                cache.push(hint(error.clone()));
                None
            }
            None => None,
        };
        if let Some(asked) = self.storage_confirm {
            // The size to free is offered only for rows that are counted.
            let counted: Vec<_> = self
                .storage_report
                .iter()
                .flat_map(|report| &report.rows)
                .filter(|r| asked.is_none_or(|category| r.category == category))
                .collect();
            let wanted = if asked.is_none() {
                Category::ALL.len()
            } else {
                1
            };
            let bytes = (counted.len() == wanted
                && counted.iter().all(|r| r.measured_bytes().is_some()))
            .then(|| counted.iter().map(|r| r.clearable_bytes()).sum())
            .filter(|&bytes| bytes > 0);
            let question = glass_storage::clear_question(asked, bytes);
            cache.push(
                div()
                    .flex()
                    .items_center()
                    .gap(px(16.))
                    .child(div().flex_1().min_w_0().child(question))
                    .child(button("alt-storage-keep", tr("Keep")).on_click(cx.listener(
                        |this, _: &ClickEvent, _, cx| {
                            this.storage_confirm = None;
                            cx.notify();
                        },
                    )))
                    .child(
                        danger(button("alt-storage-clear-yes", tr("Clear"))).on_click(cx.listener(
                            |this, _: &ClickEvent, _, cx| {
                                if let Some(clear) = this.storage_confirm.take() {
                                    this.refresh_storage(clear, clear.is_none());
                                }
                                cx.notify();
                            },
                        )),
                    ),
            );
        }
        for (n, category) in Category::ALL.into_iter().enumerate() {
            let found = self
                .storage_report
                .as_ref()
                .and_then(|report| report.rows.iter().find(|r| r.category == category));
            let size = found
                .and_then(storage::Row::measured_bytes)
                .map_or_else(|| uncounted.to_string(), storage::format_bytes);
            let clear = button(ElementId::named_usize("alt-storage-clear", n), tr("Clear…"));
            let mut line = row(
                trf(
                    "{category} · {size}",
                    &[("category", category.label()), ("size", &size)],
                ),
                glass_storage::category_about(category),
                div().child(if offered && glass_storage::row_clearable(found) {
                    clear.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.storage_confirm = Some(Some(category));
                        cx.notify();
                    }))
                } else {
                    off(clear)
                }),
            );
            let feedback = cleared.and_then(|report| {
                report
                    .rows
                    .iter()
                    .find(|r| r.category == category && r.after_clear)
            });
            if let Some(kept) = feedback
                .and_then(glass_storage::kept_line)
                .or_else(|| found.and_then(glass_storage::kept_line))
            {
                line = div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(line)
                    .child(hint(kept));
            }
            cache.push(line);
        }
        let notes = [
            self.client
                .is_some()
                .then(|| hint(tr("A game is running. Files it may use are kept."))),
            busy.then(|| hint(tr("Counting, or clearing…"))),
        ];
        div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(group(vec![self.location_row(cx)]))
            .child(self.storage_build_rows(cx))
            .child(self.storage_build_options(cx))
            .child(group(vec![files]))
            .child(group(cache))
            .children(notes.into_iter().flatten())
    }

    fn settings_privacy(&mut self, cx: &mut Context<Self>) -> Div {
        let consent = self.collection.consent.clone();
        // A row: its name with a "?" that says exactly what is sent, a line on why, the switch.
        let line = |this: &Self,
                    name: &'static str,
                    summary: &'static str,
                    full: &'static str,
                    toggle: Stateful<Div>,
                    cx: &mut Context<Self>| {
            let pinned = this.collection.sent_open.contains(&name);
            let shown = pinned || this.collection.sent_hover == Some(name);
            let help = div()
                .id(name)
                .tab_index(0)
                .seen()
                .flex_none()
                .size(px(16.))
                .rounded_full()
                .border_1()
                .border_color(rgb(if pinned {
                    palette().text
                } else {
                    palette().line
                }))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(tokens::CAPTION))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(if pinned {
                    palette().text
                } else {
                    palette().muted
                }))
                .cursor_pointer()
                .focus_visible(|s| s.border_color(rgb(palette().focus)))
                // One line the size of the ring's inside, its text centred: the "?" sits in the
                // middle both ways, whatever width its text element measures.
                .child(
                    div()
                        .w_full()
                        .text_center()
                        .line_height(px(16. - 2.))
                        .child("?"),
                )
                .on_hover(cx.listener(move |this, over: &bool, _, cx| {
                    let hover = &mut this.collection.sent_hover;
                    match *over {
                        true => *hover = Some(name),
                        false if *hover == Some(name) => *hover = None,
                        false => {}
                    }
                    cx.notify();
                }))
                // A press pins it open, for keyboards and touch; another press closes it.
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    let list = &mut this.collection.sent_open;
                    match list.contains(&name) {
                        true => list.clear(),
                        false => *list = vec![name],
                    }
                    cx.notify();
                }));
            let said = shown.then(|| {
                // Under the "?", or above its row when the row is low in the window,
                // but never on top of the "?": covering it ends the hover, which hides
                // the card, which uncovers the "?" again, and the card flickers.
                // The anchor sits just above the row. The offset puts the card below
                // the "?"; when that does not fit, gpui flips the card to end at the
                // anchor (dropping the offset), so it ends above the row.
                // The side padding keeps the card off the window edge when it is shifted.
                gpui::deferred(
                    div().absolute().top(px(-6.)).left(px(-8.)).child(
                        gpui::anchored().offset(gpui::point(px(0.), px(30.))).child(
                            div().px(px(8.)).child(
                                div()
                                    .w(px(340.))
                                    .p(px(12.))
                                    .rounded(crate::tokens::r(crate::tokens::R_CONTROL))
                                    .border_1()
                                    .border_color(palette().input_edge())
                                    .bg(rgb(palette().raised))
                                    .shadow_lg()
                                    .occlude()
                                    .child(hint(tr(full))),
                            ),
                        ),
                    ),
                )
                .with_priority(10)
            });
            div()
                .flex()
                // The switch in the middle of its row, as everywhere else in Settings.
                .items_center()
                .gap(px(16.))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(
                            div()
                                .relative()
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .child(div().font_weight(FontWeight::SEMIBOLD).child(tr(name)))
                                .child(help)
                                .children(said),
                        )
                        .child(hint(tr(summary))),
                )
                .child(toggle)
        };
        let ping = line(
            self,
            telemetry::PING_NAME,
            telemetry::PING_SUMMARY,
            telemetry::PING,
            switch("alt-ping", !self.settings.ping_off).on_click(cx.listener(
                |this, _: &ClickEvent, _, cx| {
                    this.settings.ping_off = !this.settings.ping_off;
                    this.save_settings();
                    cx.notify();
                },
            )),
            cx,
        );
        // The game reads the same switch (`backend::no_ai`).
        let no_ai_on = backend::no_ai();
        let no_ai = line(
            self,
            "No AI",
            "Hides and turns off every AI feature.",
            "Every AI feature in the launcher and the game is hidden and turned off: agent control of the launcher and the game, sunrise-ctl and the MCP servers.",
            switch("alt-no-ai", no_ai_on).on_click(cx.listener(
                move |this, _: &ClickEvent, _, cx| {
                    // A failed save is a problem like any other: the play bar says it.
                    if let Err(e) = backend::set_no_ai(!no_ai_on) {
                        this.client_error = Some(e);
                    }
                    cx.notify();
                },
            )),
            cx,
        );
        let mut reports = Vec::new();
        for (n, &(kind, name, summary, what)) in telemetry::SWITCHES.iter().enumerate() {
            let on = !consent.needs_asking() && consent.choices.allows(kind);
            let toggle = switch(ElementId::named_usize("alt-report", n), on).on_click(cx.listener(
                move |this, _: &ClickEvent, _, cx| {
                    let mut choices = if this.collection.consent.needs_asking() {
                        Choices::NONE
                    } else {
                        this.collection.consent.choices
                    };
                    choices.set(kind, !on);
                    this.answer_consent(choices);
                    cx.notify();
                },
            ));
            reports.push(line(self, name, summary, what, toggle, cx));
        }
        let id = match consent.install_id.as_str() {
            "" => tr("No install ID: nothing has been collected.").to_string(),
            id => trf(
                "Install ID {prefix}…{suffix}",
                &[("prefix", &id[..4]), ("suffix", &id[id.len() - 4..])],
            ),
        };
        let waiting = if sunrise_launcher_telemetry::endpoint().is_some() {
            let count = sunrise_launcher_telemetry::outbox::pending(&telemetry::home()).len();
            trf(
                "{count} waiting to be sent",
                &[("count", &count.to_string())],
            )
        } else {
            tr("Sending is turned off on this computer").to_string()
        };
        let confirm = self.collection.confirm_delete;
        let viewing = self.collection.view.is_some();
        let mut yours = self.wide_row(
            tr("Your data"),
            format!("{id} · {waiting}"),
            div()
                .child(
                    button(
                        "alt-data-view",
                        tr(if viewing {
                            "Hide"
                        } else {
                            "See what would be sent"
                        }),
                    )
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.toggle_view();
                        cx.notify();
                    })),
                )
                .child(
                    danger(button(
                        "alt-data-delete",
                        tr(if confirm {
                            "Delete everything?"
                        } else {
                            "Delete my data"
                        }),
                    ))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        if this.collection.confirm_delete {
                            this.collection.note = Some(match telemetry::delete_my_data() {
                                Ok(done) => done,
                                Err(e) => e,
                            });
                            this.collection.consent = telemetry::consent();
                            this.collection.view = None;
                        }
                        this.collection.confirm_delete = !this.collection.confirm_delete;
                        cx.notify();
                    })),
                ),
        );
        if let Some((what, json)) = self.collection.view.clone() {
            yours = div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .child(yours)
                .child(hint(what))
                .child(crate::code_box::code_box(
                    "alt-data-json",
                    vec![json.into()],
                    Some(220.),
                ));
        }
        if let Some(note) = self.collection.note.clone() {
            yours = div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(yours)
                .child(hint(note));
        }
        div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(group(vec![ping]))
            .child(hint(tr("These reports are off until you turn them on.")))
            .child(group(reports))
            .child(group(vec![yours]))
            .child(hint(tr(telemetry::NEVER)))
            .child(group(vec![no_ai]))
    }

    /// Preview rounding while dragging, without changing the saved choice.
    pub(super) fn corner_roundness(&self) -> f32 {
        if self.ui.page == crate::frame::Page::Settings
            && self.ui.settings.section == Section::Appearance
        {
            if let Some(value) = self.corners_slider.preview() {
                return f32::from(corner_px(value)) / f32::from(backend::DEFAULT_CORNER_RADIUS);
            }
        } else {
            self.corners_slider.cancel();
        }
        self.settings.roundness()
    }

    /// How round every corner is: a button's corner radius in px, by slider or typed; cards
    /// and panels keep their proportion to it.
    pub(super) fn corners_row(&mut self, cx: &mut Context<Self>) -> Div {
        let max = f32::from(backend::MAX_CORNER_RADIUS);
        let now = self.settings.corner_radius();
        let preview = self
            .corners_slider
            .preview()
            .map(|value| line_edit::LineEdit::new(corner_px(value).to_string(), 2, 3));
        let field = self
            .settings_field(
                "alt-corners-px",
                self.corners_focus.clone(),
                self.corners_focused,
                preview.as_ref().unwrap_or(&self.corners_input),
                "",
                |this| &mut this.corners_input,
                |this| this.save_corners(),
                cx,
            )
            .w(px(52.));
        let slider = self.corners_slider.render(
            "alt-corners",
            f32::from(now) / max,
            1. / max,
            |this, value, cx| {
                let radius = corner_px(value);
                this.settings.corner_radius_px = Some(radius);
                this.corners_input.set(radius.to_string());
                this.save_settings();
                cx.notify();
            },
            cx,
        );
        row(
            tr("Corner rounding"),
            "",
            div()
                .flex()
                .min_w_0()
                .items_center()
                .justify_end()
                .gap(px(10.))
                .child(slider.flex_1().min_w(px(80.)).max_w(px(200.)))
                .child(field.flex_none())
                .child(
                    div()
                        .flex_none()
                        .text_size(px(tokens::LABEL))
                        .text_color(rgb(palette().muted))
                        .child(tr("px")),
                ),
        )
    }

    /// The corner radius as typed: a number up to the most it goes; anything else waits.
    fn save_corners(&mut self) {
        if let Ok(radius) = self.corners_input.text().trim().parse::<u8>() {
            self.settings.corner_radius_px = Some(radius.min(backend::MAX_CORNER_RADIUS));
            self.save_settings();
        }
    }

    fn settings_launcher(&mut self, cx: &mut Context<Self>) -> Div {
        let picking = self.ui.settings.languages;
        let chosen = self.settings.language;
        let system = i18n::Language::system().name();
        let now: SharedString = match chosen {
            Some(l) => l.name().into(),
            None => tr("Same as the system").into(),
        };
        // One button with the language on it; a press opens the list under it, as Discover's
        // menus do. 87 languages don't fit on a page as buttons.
        let opener = button("alt-language", now)
            .gap(px(8.))
            .child(crate::glass_ui::fold_chevron(
                "settings-language",
                picking,
                13.,
                rgb(palette().muted).into(),
            ))
            .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.ui.settings.languages = !this.ui.settings.languages;
                cx.notify();
            }));
        let menu = picking.then(|| {
            let options = std::iter::once(None)
                .chain(i18n::Language::ALL.map(Some))
                .enumerate()
                .map(|(i, choice)| {
                    let on = choice == chosen;
                    let label: SharedString = match choice {
                        Some(l) => l.name().into(),
                        None => tr("Same as the system").into(),
                    };
                    div()
                        .id(("alt-language-option", i))
                        .reach()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .px(px(10.))
                        .py(px(7.))
                        .rounded(tokens::r(tokens::R_SMALL))
                        .text_size(px(tokens::LABEL))
                        .cursor_pointer()
                        .hover(|s| s.bg(rgb(palette().hover)))
                        .when(on, |d| d.font_weight(FontWeight::SEMIBOLD))
                        .child(div().flex_1().min_w_0().truncate().child(label))
                        // The system's own language, beside the choice that follows it.
                        .when(choice.is_none(), |d| {
                            d.child(
                                div()
                                    .flex_none()
                                    .text_color(rgb(palette().muted))
                                    .child(system),
                            )
                        })
                        .when(on, |d| {
                            d.child(crate::blocks::icon(
                                "check",
                                13.,
                                rgb(palette().text).into(),
                            ))
                        })
                        // The first choice stands apart from the languages under it.
                        .when(choice.is_none(), |d| {
                            d.mb(px(4.)).border_b_1().border_color(rgb(palette().line))
                        })
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.settings.language = choice;
                            i18n::set(this.settings.language());
                            this.save_settings();
                            this.ui.settings.languages = false;
                            cx.notify();
                        }))
                });
            let list = div()
                .id("alt-language-menu")
                .w(px(260.))
                .max_h(px(320.))
                .overflow_y_scroll()
                .p(px(6.))
                .rounded(tokens::r(tokens::R_CONTROL))
                .border_1()
                .border_color(palette().input_edge())
                .bg(rgb(palette().raised))
                .shadow_lg()
                .occlude()
                .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_scroll(own_wheel)
                .flex()
                .flex_col()
                .children(options);
            // Under the button, right-aligned to it, and kept inside the window: at 800×600 the
            // whole list, edge included, moves up rather than running off the bottom.
            gpui::deferred(
                div()
                    .absolute()
                    .top(px(tokens::CONTROL + 6.))
                    .right_0()
                    .child(
                        gpui::anchored()
                            .anchor(gpui::Anchor::TopRight)
                            .snap_to_window_with_margin(px(8.))
                            .child(list),
                    ),
            )
            .with_priority(10)
        });
        let language = row(
            tr("Language"),
            "",
            div().relative().child(opener).children(menu),
        );
        fit_field(&mut self.proxy_input, self.window_width);
        let refused = (!self.proxy_focused)
            .then(|| sunrise_launcher::proxy::refuse(self.proxy_input.text()))
            .flatten();
        let field = self.settings_field(
            "alt-proxy",
            self.proxy_focus.clone(),
            self.proxy_focused,
            &self.proxy_input,
            "For example http://127.0.0.1:7890",
            |this| &mut this.proxy_input,
            |this| this.save_proxy(),
            cx,
        );
        let mut proxy = stacked(
            tr("Proxy"),
            tr("Downloads, sign-ins and updates go through it. Empty means none; HTTPS_PROXY then applies if the launcher was started with one."),
            field,
        );
        if let Some(why) = refused {
            proxy = proxy.child(warning(why));
        }
        let version = self.launcher_update_row(cx);
        div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(group(vec![language]))
            .child(group(vec![proxy]))
            .child(group(vec![version]))
    }

    /// This launcher's version and its own update: the state of the staged download, and the
    /// action that fits it (Update, or Download again when this copy can't replace itself).
    fn launcher_update_row(&mut self, cx: &mut Context<Self>) -> Div {
        let title = trf(
            "Sunrise Launcher {version}",
            &[("version", backend::VERSION)],
        );
        use crate::launcher_update::Status;
        let recovery = || {
            self.launcher_download_available().then(|| {
                let url = backend::download_page(&self.links.website);
                button("alt-launcher-download", tr("Download again"))
                    .on_click(move |_, _, cx| cx.open_url(&url))
            })
        };
        let offer = self.launcher_update_offer();
        let retry_download = offer
            .as_ref()
            .is_some_and(|offer| matches!(offer.status, Status::Failed(_)))
            .then(recovery)
            .flatten();
        let rejected = matches!(
            self.updates.launcher_error,
            Some(backend::LauncherCheckError::Rejected { .. })
        );
        let verification_error = || warning(tr("The launcher update could not be verified."));
        let (about, action, bad) = if let Some(offer) = &offer {
            let armed = self.restart_for_launcher_update;
            let action = if armed {
                Some(
                    button("alt-launcher-cancel", tr("Cancel restart")).on_click(cx.listener(
                        |this, _, _, cx| {
                            this.cancel_launcher_restart();
                            cx.notify();
                        },
                    )),
                )
            } else if let Some(label) = offer.action(self.launcher_update_wait(), armed) {
                Some(button("alt-launcher-update", label).on_click(cx.listener(
                    |this, _, _, cx| {
                        this.request_launcher_update();
                        cx.notify();
                    },
                )))
            } else if matches!(offer.status, Status::Stuck(_)) {
                recovery()
            } else {
                None
            };
            let available = offer.about(self);
            let about = match offer.status {
                Status::Available | Status::Downloading(_) => available,
                _ => trf(
                    "{version} available · {status}",
                    &[("version", offer.version), ("status", &available)],
                ),
            };
            (
                about,
                action,
                matches!(offer.status, Status::Failed(_) | Status::Stuck(Some(_))),
            )
        } else if let SelfUpdate::Stuck(error) = &self.self_update {
            (
                self.launcher_update_help(error.as_deref()),
                recovery(),
                error.is_some(),
            )
        } else if self.checking_updates {
            (
                String::new(),
                Some(off(button("alt-launcher-check", tr("Checking…")))),
                false,
            )
        } else {
            let (about, bad) = if rejected {
                (String::new(), true)
            } else {
                launcher_check_line(
                    self.updates
                        .launcher_error
                        .as_ref()
                        .map(backend::LauncherCheckError::message),
                    self.launcher_checked_now,
                    self.checked.is_some(),
                )
            };
            let check = button("alt-launcher-check", tr("Check now")).on_click(cx.listener(
                |this, _, _, cx| {
                    this.check_updates(UpdateCheck::ManualLauncher);
                    cx.notify();
                },
            ));
            (about, Some(check), bad)
        };
        let mut words = div().flex().flex_col().gap(px(2.));
        words = words.child(div().font_weight(FontWeight::SEMIBOLD).child(title));
        if !about.is_empty() {
            words = words.child(if bad { warning(about) } else { hint(about) });
        }
        div()
            .flex()
            .items_center()
            .gap(px(16.))
            .child(
                words
                    .flex_1()
                    .min_w_0()
                    .children(rejected.then(verification_error)),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .flex_col()
                    .items_end()
                    .gap(px(6.))
                    .children(action)
                    .children(retry_download),
            )
    }

    /// One line of text in this look, saved as it is typed.
    #[allow(clippy::too_many_arguments)]
    fn settings_field(
        &self,
        id: &'static str,
        focus: FocusHandle,
        focused: bool,
        input: &line_edit::LineEdit,
        placeholder: &'static str,
        get: fn(&mut Self) -> &mut line_edit::LineEdit,
        edited: fn(&mut Self),
        cx: &mut Context<Self>,
    ) -> Div {
        let theme = crate::mods::field_theme(self.theme);
        let to_focus = focus.clone();
        let field = crate::frame::input(focused)
            .id(id)
            .track_focus(&focus)
            .seen()
            .h(px(tokens::FIELD))
            .px(px(12.))
            .font_family(theme::MONO)
            .text_size(px(tokens::LABEL))
            .text_color(rgb(palette().text))
            .cursor_text()
            .child(line_edit::content(
                input,
                &theme,
                focused,
                false,
                false,
                placeholder,
            ))
            .on_click(cx.listener(move |_, _: &ClickEvent, window, cx| {
                window.focus(&to_focus, cx);
                cx.notify();
            }))
            .on_key_down(cx.listener(move |this, ev: &KeyDownEvent, window, cx| {
                match line_edit::handle(get(this), ev, cx) {
                    line_edit::Key::Pass => return,
                    line_edit::Key::Escape | line_edit::Key::Enter => window.blur(cx),
                    line_edit::Key::Edited => edited(this),
                    _ => {}
                }
                cx.stop_propagation();
                cx.notify();
            }));
        div().child(line_edit::mouse(field, cx, get))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_data_folder_problem_opens_settings_on_storage() {
        assert!(startup_section(true, false) == Some(Section::Storage));
        assert!(startup_section(false, true) == Some(Section::Storage));
        assert!(startup_section(false, false).is_none());
    }

    #[test]
    fn a_text_box_shows_as_much_as_its_pane_is_wide() {
        assert_eq!(field_chars(1280.), 91);
        assert_eq!(field_chars(900.), 44);
        assert_eq!(field_chars(300.), 21);
    }

    #[test]
    fn a_long_proxy_shows_whole_in_a_wide_window_and_the_settings_view_sizes_its_box() {
        let line = "http://user:password@proxy.example.org:7890/".repeat(2);
        let mut input = line_edit::LineEdit::new(line.clone(), 256, 44);
        fit_field(&mut input, 1280.);
        assert_eq!(input.text(), line);
        assert!(line.chars().count() <= 91);
        assert_eq!(input.shown(), line);
        // The view sizes the proxy box through `fit_field`; a Launcher needs a window to build.
        let source = include_str!("settings.rs");
        assert!(source.contains("fit_field(&mut self.proxy_input, self.window_width);"));
    }

    #[test]
    fn the_launcher_row_says_what_its_check_found() {
        let now = Some(Instant::now());
        // A failure is shown as one and is retryable (the row keeps its button); it is never
        // passed off as up to date, even right after a check.
        let raw = "No connection: https://api.github.com/repos/sunrise-org/sunrise-launcher/releases?per_page=100 Dns Failed: resolve dns name 'api.github.com:443', and Sunrise's mirror of GitHub couldn't help either (http://127.0.0.1:40319/repos/x answered 500)";
        let (line, bad) = launcher_check_line(Some(raw), now, true);
        assert!(bad);
        for leaked in ["http", "127.0.0.1", "40319", "api.github.com", "500", "Dns"] {
            assert!(!line.contains(leaked), "{leaked} in {line}");
        }
        assert_eq!(
            line,
            "Couldn't check for updates. Check your connection and try again."
        );
        // The detail goes to the log instead.
        let mut logged = String::new();
        log_launcher_check_failure(raw, |l| logged = l.to_string());
        assert!(
            logged.contains(raw) && logged.contains("127.0.0.1:40319"),
            "{logged}"
        );
        assert_eq!(
            launcher_check_line(None, now, true),
            ("up to date · checked just now".into(), false)
        );
        let long_ago = Instant::now().checked_sub(Duration::from_secs(120));
        assert_eq!(
            launcher_check_line(None, long_ago, true),
            ("up to date".into(), false)
        );
        assert_eq!(
            launcher_check_line(None, None, false),
            (String::new(), false)
        );
    }
}
