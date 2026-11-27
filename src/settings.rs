//! Settings for the whole launcher, one section at a time. The game build and its channel stay
//! in the play bar, the one place that shows them.
use super::*;
use crate::frame::Reach as _;
use crate::paint::palette;
use crate::widgets::check::switch;
use crate::widgets::choice::choices;
use crate::widgets::feedback::{banner, Tone};
use crate::widgets::nav::{nav_row, sun_fits};
use gpui::rgb;
use storage::Category;
use sunrise_launcher::telemetry;
use sunrise_launcher_telemetry::Choices;

mod account_section;
mod appearance;
mod game;
mod launcher;
mod privacy;
mod proxy;
mod storage_section;

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
/// What a section row has for its label and the sun: the row's width less its padding (12 px a
/// side), its icon (20 px) and the gap after the icon (10 px).
const NAV_LABEL_ROOM: f32 = NAV_WIDTH - 24. - 20. - 10.;

/// A section label's width at full length, roughly: Geist at the label size is about 0.6 em a
/// character, which errs wide, so the sun is not drawn over a label that is close to the edge.
fn nav_label_width(label: &str) -> f32 {
    label.chars().count() as f32 * tokens::LABEL * 0.6
}

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
        crate::i18n::tr_key(match self {
            Section::Account => "Account",
            Section::Game => "Game",
            Section::Appearance => "Appearance",
            Section::Storage => "Storage",
            Section::Privacy => "Privacy",
            Section::Launcher => "Launcher",
        })
    }

    /// Its icon in the section list (a name in the glass icon set).
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
        crate::i18n::tr_key(match self {
            Section::Account => "Who plays, and the Microsoft accounts signed in here.",
            Section::Game => "How the game starts. Its build is chosen beside Play on Home.",
            Section::Appearance => "The theme, its accent and the Home banner.",
            Section::Storage => "Manage game builds, downloads and where your files are kept.",
            Section::Privacy => "What Sunrise sends, and what it never does.",
            Section::Launcher => "Language, network and this version.",
        })
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
}

/// A card of rows with a rule between them.
pub(crate) fn group(rows: Vec<Div>) -> Div {
    let mut card = div()
        .flex()
        .flex_col()
        .rounded(crate::tokens::r(crate::tokens::R_CARD))
        .border_1()
        .border_color(palette().rule())
        .bg(palette().card());
    for (i, row) in rows.into_iter().enumerate() {
        card = card.child(
            row.id(("settings-row", i))
                .px(px(16.))
                .py(px(14.))
                .when(i > 0, |d| d.border_t_1().border_color(palette().rule())),
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
        // The title and the section list stay in view as the section scrolls (`Sticky`, which
        // reads the columns row as the scroll's item 0).
        let column = div()
            .relative()
            .top(px(self.settings_sticky.shift(&scroll, 0, 0)))
            .flex()
            .flex_col()
            .gap(px(16.))
            .w(px(NAV_WIDTH))
            .flex_none()
            .child(
                div()
                    .px(px(12.))
                    .text_size(px(tokens::DISPLAY))
                    .font_weight(FontWeight::BOLD)
                    .child(tr("Settings")),
            )
            .child(self.settings_nav(section, cx));
        let body = match section {
            Section::Account => self.settings_account(cx),
            Section::Game => self.settings_game(cx),
            Section::Appearance => self.settings_appearance(window, cx),
            Section::Storage => self.settings_storage(cx),
            Section::Privacy => self.settings_privacy(cx),
            Section::Launcher => self.settings_launcher(cx),
        };
        let pane = self.settings_pane(section, body);
        div()
            .size_full()
            .flex()
            .flex_col()
            .text_color(rgb(palette().text))
            .child(crate::scrollbar::wrap_page(
                "settings",
                div()
                    .id("alt-settings-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(px(32.))
                    .pt(px(24.))
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
                            .child(column)
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

    /// The open section on the right: its title and what it is for, the warning about a data
    /// folder that did not open, then its groups.
    fn settings_pane(&self, section: Section, body: Div) -> Div {
        div()
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
            .children(
                self.location_error
                    .clone()
                    .map(|why| banner(Tone::Danger, why, None)),
            )
            .child(body)
    }

    /// The section list: one row per section, the open one filled and marked with the pixel sun
    /// when its label and the sun fit the row.
    fn settings_nav(&self, section: Section, cx: &mut Context<Self>) -> Div {
        let mut nav = div().flex().flex_col().gap(px(2.));
        for (n, s) in Section::ALL.into_iter().enumerate() {
            let on = s == section;
            let label = tr(s.label());
            let sun = on && sun_fits(nav_label_width(label), NAV_LABEL_ROOM);
            nav = nav.child(
                nav_row(format!("alt-settings-nav-{n}"), s.icon(), label, on, sun).on_click(
                    cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.settings_open(s);
                        cx.notify();
                    }),
                ),
            );
        }
        nav
    }

    /// The width of the section pane at the current window and sidebar, before anything has been
    /// measured: the page's content width less the scroll area's margins (32 px a side, where
    /// the page's own are 24) and the section list beside it, up to the pane's widest.
    pub(super) fn settings_pane_width(&self) -> f32 {
        let room = (self.content_width() - 16.).min(COLUMNS);
        (room - NAV_WIDTH - COLUMN_GAP).clamp(0., PANE_WIDTH)
    }

    /// Opens a section, leaving no question from another one standing.
    pub(super) fn settings_open(&mut self, section: Section) {
        // Another section starts at its top, with its list of sections back in place: the old
        // section's scroll would cut the new one's title off, and the list would hold on to
        // where the old one had pushed it.
        if self.ui.settings.section != section {
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
            self.refresh_open_storage_builds();
            self.refresh_storage(None, false);
        }
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
        let field = crate::widgets::field::field_box(focused, false)
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
mod tests;
